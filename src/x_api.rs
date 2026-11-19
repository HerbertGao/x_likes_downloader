use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::Config;
use crate::error::{ErrorKind, ErrorPayload};

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

    /// Fetch the `TweetDetail` GraphQL response for a single tweet.
    ///
    /// 返回 `(http_status, body)`。构造/传输错误分类与禁止泄漏 header 的约束见
    /// [`Self::graphql_get`]（多个 GraphQL GET 共用该机械）。
    pub async fn get_tweet_detail(
        &self,
        tweet_id: &str,
    ) -> std::result::Result<(u16, Value), ErrorPayload> {
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

        self.graphql_get(
            &self.config.tweet_detail_api_url,
            &variables,
            &self.config.tweet_features,
            &self.config.tweet_fieldtoggles,
        )
        .await
    }

    /// Fetch the `UserByScreenName` GraphQL response for a username.
    ///
    /// 返回 `(http_status, body)`；用户不存在时 X 返回 HTTP 200 + `data` 缺 `user` 键，
    /// 由上层（`agent::user_tweets`）分类为 `tweet_unavailable`。
    pub async fn get_user_by_screen_name(
        &self,
        screen_name: &str,
    ) -> std::result::Result<(u16, Value), ErrorPayload> {
        let variables = json!({ "screen_name": screen_name });

        self.graphql_get(
            &self.config.user_by_screen_name_api_url,
            &variables,
            &self.config.user_by_screen_name_features,
            &self.config.user_by_screen_name_fieldtoggles,
        )
        .await
    }

    /// Fetch one page of the `UserTweets` GraphQL response for a user id.
    ///
    /// 返回 `(http_status, body)`；翻页由上层按 `cursor-bottom-` 游标驱动，
    /// 单页条数走 `count`（默认沿用配置值）。
    pub async fn get_user_tweets(
        &self,
        user_id: &str,
        count: i64,
        cursor: Option<&str>,
    ) -> std::result::Result<(u16, Value), ErrorPayload> {
        let mut variables = json!({
            "userId": user_id,
            "count": count,
            "includePromotedContent": false,
            "withQuickPromoteEligibilityTweetFields": false,
            "withVoice": true
        });
        if let Some(c) = cursor {
            variables["cursor"] = json!(c);
        }

        self.graphql_get(
            &self.config.user_tweets_api_url,
            &variables,
            &self.config.user_tweets_features,
            &self.config.user_tweets_fieldtoggles,
        )
        .await
    }

    /// 鉴权 GraphQL GET 的公共机械：拼 URL（variables/features/fieldToggles 均 URL 编码）、
    /// 构造 headers（Bearer / Cookie / CSRF / 可选 UA）、发送并读取 body。
    ///
    /// 错误分两类：HTTP 发起前的构造失败（序列化 / header 解析）→ `internal_error`
    /// （非网络、不可重试）；HTTP 发起后的传输失败（连接 / 超时 / 读取）→ `network_error`。
    /// 不把构造失败误标成可重试的 network_error。
    /// 返回 `(http_status, body)`：先取状态码再读 body，仅传输错误返 `Err`（供上层映射
    /// network_error）；非 JSON body（如 401 HTML 页）解析为 `Value::Null` 而非 `Err`，
    /// 非 2xx 不提前 `Err`，让上层分类器靠状态码正确判定。
    /// 禁止把含 cookie/bearer 的 header 或完整 URL 打到 stderr。
    async fn graphql_get(
        &self,
        api_url: &str,
        variables: &Value,
        features: &str,
        fieldtoggles: &str,
    ) -> std::result::Result<(u16, Value), ErrorPayload> {
        let variables_str = serde_json::to_string(variables).map_err(|e| {
            ErrorPayload::new(
                ErrorKind::InternalError,
                format!("GraphQL variables 序列化失败: {e}"),
            )
        })?;
        let variables_encoded = urlencoding::encode(&variables_str);
        let features_encoded = urlencoding::encode(features);
        let fieldtoggles_encoded = urlencoding::encode(fieldtoggles);

        let url = format!(
            "{}?variables={}&features={}&fieldToggles={}",
            api_url, variables_encoded, features_encoded, fieldtoggles_encoded
        );

        let header_err = |name: &str| {
            let name = name.to_string();
            move |e| {
                ErrorPayload::new(
                    ErrorKind::InternalError,
                    format!("{name} header 构造失败: {e}"),
                )
            }
        };
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.config.bearer_token)
                .parse()
                .map_err(header_err("Authorization"))?,
        );
        headers.insert(
            "Cookie",
            format!(
                "auth_token={}; ct0={}",
                self.config.auth_token, self.config.ct0
            )
            .parse()
            .map_err(header_err("Cookie"))?,
        );
        headers.insert(
            "X-Csrf-Token",
            self.config
                .ct0
                .parse()
                .map_err(header_err("X-Csrf-Token"))?,
        );
        if !self.config.user_agent.is_empty() {
            headers.insert(
                "User-Agent",
                self.config
                    .user_agent
                    .parse()
                    .map_err(header_err("User-Agent"))?,
            );
        }

        let response = self
            .client
            .get(&url)
            .headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| {
                ErrorPayload::new(ErrorKind::NetworkError, format!("GraphQL 请求失败: {e}"))
            })?;

        let status = response.status().as_u16();
        let text = response.text().await.map_err(|e| {
            ErrorPayload::new(
                ErrorKind::NetworkError,
                format!("读取 GraphQL 响应失败: {e}"),
            )
        })?;
        let resp = serde_json::from_str(&text).unwrap_or(Value::Null);

        Ok((status, resp))
    }
}
