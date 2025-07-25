use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::config::Config;

pub struct Downloader {
    client: Client,
    config: Config,
    downloaded_ids: HashSet<String>,
}

impl Downloader {
    pub fn new(config: Config) -> Result<Self> {
        let client_builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30));

        let client = client_builder.build()?;
        let downloaded_ids = Self::load_downloaded_ids(&config.download_record)?;

        Ok(Downloader {
            client,
            config,
            downloaded_ids,
        })
    }

    fn load_downloaded_ids(filename: &str) -> Result<HashSet<String>> {
        if !Path::new(filename).exists() {
            return Ok(HashSet::new());
        }

        let content = fs::read_to_string(filename)
            .with_context(|| format!("无法读取文件: {}", filename))?;

        Ok(content.lines().map(|s| s.trim().to_string()).collect())
    }

    pub fn save_downloaded_id(&mut self, tweet_id: &str) -> Result<()> {
        if let Some(parent) = Path::new(&self.config.download_record).parent() {
            fs::create_dir_all(parent)?;
        }

        use std::io::Write;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.download_record)?
            .write_all(format!("{}\n", tweet_id).as_bytes())?;

        self.downloaded_ids.insert(tweet_id.to_string());
        Ok(())
    }

    pub fn is_downloaded(&self, tweet_id: &str) -> bool {
        self.downloaded_ids.contains(tweet_id)
    }

    pub fn downloaded_count(&self) -> usize {
        self.downloaded_ids.len()
    }

    pub async fn call_media_downloader(&self, tweet: &Value, tweet_id: &str) -> Result<Option<bool>> {
        // 从 tweet 对象中抽取推文主体
        let tweet_obj = self.extract_tweet_object(tweet)?;
        if tweet_obj.is_none() {
            return Ok(None);
        }
        let tweet_obj = tweet_obj.unwrap();

        // 提取作者用户名
        let username = self.extract_username(&tweet_obj)?;

        // 提取媒体列表
        let media_urls = self.extract_media_urls(&tweet_obj)?;
        if media_urls.is_empty() {
            return Ok(None);
        }

        // 构建文件名前缀
        let prefix = self.config.file_format
            .replace("{USERNAME}", &username.unwrap_or_default())
            .replace("{ID}", tweet_id)
            .replace(" ", "_");

        let output_dir = Path::new(&self.config.download_dir);
        fs::create_dir_all(output_dir)?;

        let mut download_success_count = 0;
        let total_media_count = media_urls.len();
        let mut skipped_count = 0;

        for (i, media_url) in media_urls.iter().enumerate() {
            let parsed_url = Url::parse(media_url)?;
            let original_name = parsed_url.path_segments()
                .and_then(|segments| segments.last())
                .unwrap_or("unknown");
            
            let filename = format!("{}_{}", prefix, original_name);
            let out_path = output_dir.join(&filename);

            // 检查文件是否已存在
            if out_path.exists() {
                let metadata = fs::metadata(&out_path)?;
                if metadata.len() > 0 {
                    println!("跳过已存在的文件 ({}/{}): {:?} (大小: {} 字节)", 
                        i + 1, total_media_count, out_path, metadata.len());
                    skipped_count += 1;
                    download_success_count += 1;
                    continue;
                } else {
                    println!("发现损坏的空文件，将重新下载 ({}/{}): {:?}", 
                        i + 1, total_media_count, out_path);
                    fs::remove_file(&out_path)?;
                }
            }

            match self.download_media(media_url, &out_path, i + 1, total_media_count).await {
                Ok(true) => {
                    download_success_count += 1;
                }
                Ok(false) => {
                    // 下载失败，继续下一个
                }
                Err(e) => {
                    println!("下载异常 ({}/{}): {} 错误: {}", i + 1, total_media_count, media_url, e);
                }
            }
        }

        if download_success_count > 0 {
            if skipped_count > 0 {
                println!("Tweet {} 处理完成: 跳过 {} 个已存在文件，成功下载 {} 个新文件", 
                    tweet_id, skipped_count, download_success_count - skipped_count);
            } else {
                println!("Tweet {} 成功下载了 {}/{} 个媒体文件", 
                    tweet_id, download_success_count, total_media_count);
            }
            Ok(Some(true))
        } else {
            println!("Tweet {} 所有媒体文件下载失败", tweet_id);
            Ok(Some(false))
        }
    }

    fn extract_tweet_object<'a>(&self, tweet: &'a Value) -> Result<Option<&'a Value>> {
        // 路径1: content.itemContent.tweet_results.result.tweet
        if let Some(tweet_obj) = tweet
            .get("content")
            .and_then(|c| c.get("itemContent"))
            .and_then(|ic| ic.get("tweet_results"))
            .and_then(|tr| tr.get("result"))
            .and_then(|r| r.get("tweet"))
        {
            return Ok(Some(tweet_obj));
        }

        // 路径2: tweet (直接结构)
        if let Some(tweet_obj) = tweet.get("tweet") {
            return Ok(Some(tweet_obj));
        }

        // 路径3: 如果都没有，使用原始tweet
        Ok(Some(tweet))
    }

    fn extract_username(&self, tweet_obj: &Value) -> Result<Option<String>> {
        let username = tweet_obj
            .get("core")
            .and_then(|c| c.get("user_results"))
            .and_then(|ur| ur.get("result"))
            .and_then(|r| r.get("legacy"))
            .and_then(|l| l.get("screen_name"))
            .and_then(|s| s.as_str());

        Ok(username.map(|s| s.to_string()))
    }

    fn extract_media_urls(&self, tweet_obj: &Value) -> Result<Vec<String>> {
        let mut media_urls = Vec::new();

        let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);
        
        // 从 extended_entities 或 entities 中提取媒体列表
        let media_list = legacy
            .get("extended_entities")
            .and_then(|ee| ee.get("media"))
            .and_then(|m| m.as_array())
            .or_else(|| {
                legacy
                    .get("entities")
                    .and_then(|e| e.get("media"))
                    .and_then(|m| m.as_array())
            });

        if let Some(media_array) = media_list {
            for media in media_array {
                let media_type = media.get("type").and_then(|t| t.as_str()).unwrap_or("");
                
                match media_type {
                    "video" => {
                        if let Some(video_info) = media.get("video_info") {
                            if let Some(variants) = video_info.get("variants").and_then(|v| v.as_array()) {
                                let best_variant = variants
                                    .iter()
                                    .filter(|v| v.get("bitrate").is_some())
                                    .max_by_key(|v| v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0));
                                
                                if let Some(variant) = best_variant {
                                    if let Some(url) = variant.get("url").and_then(|u| u.as_str()) {
                                        media_urls.push(url.to_string());
                                    }
                                }
                            }
                        }
                    }
                    "photo" => {
                        if let Some(url) = media.get("media_url_https").and_then(|u| u.as_str()) {
                            media_urls.push(url.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(media_urls)
    }

    async fn download_media(&self, url: &str, out_path: &Path, current: usize, total: usize) -> Result<bool> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("User-Agent", self.config.user_agent.parse()?);

        let response = self.client
            .get(url)
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            println!("下载失败 ({}) ({}/{}): {}", response.status(), current, total, url);
            return Ok(false);
        }

        let content_length = response.content_length();
        let mut downloaded_size = 0u64;

        // 创建进度条
        let pb = ProgressBar::new(content_length.unwrap_or(0));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        let mut file = File::create(out_path).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded_size += chunk.len() as u64;
            pb.set_position(downloaded_size);
        }

        pb.finish_with_message("下载完成");

        // 验证下载完整性
        if let Some(expected_size) = content_length {
            if downloaded_size != expected_size {
                println!("下载不完整 ({}/{}): {:?} (期望: {}, 实际: {})", 
                    current, total, out_path, expected_size, downloaded_size);
                fs::remove_file(out_path)?;
                return Ok(false);
            }
        }

        println!("下载成功 ({}/{}): {:?} (大小: {} 字节)", 
            current, total, out_path, downloaded_size);
        Ok(true)
    }
} 