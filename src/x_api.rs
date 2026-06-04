use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::Config;

#[derive(Debug)]
pub struct XApi {
    client: Client,
    config: Config,
}

impl XApi {
    pub fn new(config: Config) -> Result<Self> {
        let client = reqwest::Client::builder().build()?;

        Ok(XApi { client, config })
    }

    pub async fn get_liked_tweets_internal(&self) -> Result<Vec<Value>> {
        let mut all_tweets = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut variables = json!({
                "userId": self.config.user_id,
                "count": self.config.count.parse::<i32>()?,
                "includePromotedContent": false,
                "withClientEventToken": false,
                "withBirdwatchNotes": false,
                "withVoice": true,
                "withV2Timeline": true
            });

            if let Some(ref cursor_val) = cursor {
                variables["cursor"] = json!(cursor_val);
            }

            let variables_str = serde_json::to_string(&variables)?;
            let variables_encoded = urlencoding::encode(&variables_str);
            let features_encoded = urlencoding::encode(&self.config.likes_features);
            let fieldtoggles_encoded = urlencoding::encode(&self.config.likes_fieldtoggles);

            let url = format!(
                "{}?variables={}&features={}&fieldToggles={}",
                self.config.likes_api_url,
                variables_encoded,
                features_encoded,
                fieldtoggles_encoded
            );

            eprintln!("请求 URL: {}", url);

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "Authorization",
                format!("Bearer {}", self.config.bearer_token).parse()?,
            );
            headers.insert(
                "Cookie",
                format!(
                    "auth_token={}; ct0={}",
                    self.config.auth_token, self.config.ct0
                )
                .parse()?,
            );
            headers.insert("X-Csrf-Token", self.config.ct0.parse()?);
            headers.insert("User-Agent", self.config.user_agent.parse()?);

            eprintln!("请求 headers: {:?}", headers);

            let response = self.client.get(&url).headers(headers).send().await?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await?;
                return Err(anyhow::anyhow!("API 请求失败: {} {}", status, text));
            }

            let data: Value = response.json().await?;

            let (tweets, new_cursor) = self.parse_likes_response(&data)?;
            eprintln!(
                "本页获取到 {} 条 tweet，cursor: {:?}",
                tweets.len(),
                new_cursor
            );

            all_tweets.extend(tweets);

            // 如果未启用 ALL 模式，则只返回第一页数据
            if !self.config.all {
                break;
            }

            // 如果没有新的 cursor 或新 cursor 与上一次相同，则认为没有更多数据
            if new_cursor.is_none() || new_cursor == cursor {
                break;
            }
            cursor = new_cursor;
        }

        Ok(all_tweets)
    }

    /// Fetch the `TweetDetail` GraphQL response for a single tweet.
    ///
    /// 返回 `(http_status, body)`。鉴权 header 复用与 Likes 一致的构造；
    /// 先取 HTTP 状态码再读 body，仅传输错误返 `Err`（供上层映射 network_error）；
    /// 非 JSON body（如 401 HTML 页）解析为 `Value::Null` 而非 `Err`，非 2xx 不提前 `Err`，
    /// 让上层分类器靠状态码正确判定。禁止把含 cookie/bearer 的 header 或完整 URL 打到 stderr。
    pub async fn get_tweet_detail(&self, tweet_id: &str) -> Result<(u16, Value)> {
        let variables = json!({
            "focalTweetId": tweet_id,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": true,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true
        });

        let variables_str = serde_json::to_string(&variables)?;
        let variables_encoded = urlencoding::encode(&variables_str);
        let features_encoded = urlencoding::encode(&self.config.tweet_features);
        let fieldtoggles_encoded = urlencoding::encode(&self.config.tweet_fieldtoggles);

        let url = format!(
            "{}?variables={}&features={}&fieldToggles={}",
            self.config.tweet_detail_api_url,
            variables_encoded,
            features_encoded,
            fieldtoggles_encoded
        );

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.config.bearer_token).parse()?,
        );
        headers.insert(
            "Cookie",
            format!(
                "auth_token={}; ct0={}",
                self.config.auth_token, self.config.ct0
            )
            .parse()?,
        );
        headers.insert("X-Csrf-Token", self.config.ct0.parse()?);
        if !self.config.user_agent.is_empty() {
            headers.insert("User-Agent", self.config.user_agent.parse()?);
        }

        let response = self
            .client
            .get(&url)
            .headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;

        let status = response.status().as_u16();
        let text = response.text().await?;
        let resp = serde_json::from_str(&text).unwrap_or(Value::Null);

        Ok((status, resp))
    }

    fn parse_likes_response(&self, data: &Value) -> Result<(Vec<Value>, Option<String>)> {
        let mut tweets = Vec::new();
        let mut new_cursor = None;

        if let Some(instructions) = data
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("result"))
            .and_then(|r| r.get("timeline_v2"))
            .and_then(|t| t.get("timeline"))
            .and_then(|t| t.get("instructions"))
            .and_then(|i| i.as_array())
        {
            for instruction in instructions {
                if instruction.get("type") == Some(&json!("TimelineAddEntries")) {
                    if let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) {
                        for entry in entries {
                            if let Some(entry_id) = entry.get("entryId").and_then(|id| id.as_str())
                            {
                                if entry_id.starts_with("tweet-") {
                                    tweets.push(entry.clone());
                                } else if entry_id.starts_with("cursor-bottom-") {
                                    new_cursor = entry
                                        .get("content")
                                        .and_then(|c| c.get("value"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok((tweets, new_cursor))
    }
}
