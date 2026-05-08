use chrono::Utc;
use serde_json::json;
use std::time::Duration;

use crate::config::Config;

use super::types::AuthStatus;

pub async fn auth_status() -> AuthStatus {
    let config = match Config::load() {
        Ok(c) => c,
        Err(_) => return AuthStatus::NotConfigured,
    };

    if !config.is_configured() {
        return AuthStatus::NotConfigured;
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return AuthStatus::NetworkError {
                message: e.to_string(),
            }
        }
    };

    // Build the lightest possible Likes request: count=1, no cursor.
    let variables = json!({
        "userId": config.user_id,
        "count": 1,
        "includePromotedContent": false,
        "withClientEventToken": false,
        "withBirdwatchNotes": false,
        "withVoice": true,
        "withV2Timeline": true
    });
    let variables_str = match serde_json::to_string(&variables) {
        Ok(s) => s,
        Err(e) => {
            return AuthStatus::NetworkError {
                message: e.to_string(),
            }
        }
    };
    let url = format!(
        "{}?variables={}&features={}&fieldToggles={}",
        config.likes_api_url,
        urlencoding::encode(&variables_str),
        urlencoding::encode(&config.likes_features),
        urlencoding::encode(&config.likes_fieldtoggles),
    );

    let headers = match build_headers(&config) {
        Ok(h) => h,
        Err(s) => return s,
    };

    let response = match client.get(&url).headers(headers).send().await {
        Ok(r) => r,
        Err(e) => {
            return AuthStatus::NetworkError {
                message: e.to_string(),
            }
        }
    };

    let status = response.status().as_u16();
    let retry_after = response_retry_after(response.headers());
    if let Some(s) = classify_auth_response_status(status, retry_after) {
        return s;
    }

    // Healthy 路径：验证响应确实解析为 Likes timeline；防御 200 返回 HTML 反爬页面。
    let body: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(_) => {
            return AuthStatus::EndpointStale;
        }
    };
    if body
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("result"))
        .and_then(|r| r.get("timeline_v2"))
        .is_none()
    {
        return AuthStatus::EndpointStale;
    }
    AuthStatus::Healthy {
        checked_at: Utc::now().to_rfc3339(),
    }
}

/// Pure helper：根据 HTTP 状态码（与 retry-after header）派生 AuthStatus。
/// 单元可测；不发请求。返回 `None` 表示状态码看起来健康，调用方仍需检查 body。
fn classify_auth_response_status(status: u16, retry_after: Option<u64>) -> Option<AuthStatus> {
    match crate::error::classify_status(status) {
        None => None, // healthy 路径需要进一步检查 body，由调用方处理
        Some(crate::error::ErrorKind::AuthExpired) => Some(AuthStatus::AuthExpired),
        Some(crate::error::ErrorKind::EndpointStale) => Some(AuthStatus::EndpointStale),
        Some(crate::error::ErrorKind::RateLimited) => Some(AuthStatus::RateLimited { retry_after }),
        Some(crate::error::ErrorKind::NetworkError) => Some(AuthStatus::NetworkError {
            message: format!("HTTP {}", status),
        }),
        // 5xx / 其它非 auth、非 404 失败：作为可重试 network 失败而非 endpoint stale
        Some(_) => Some(AuthStatus::NetworkError {
            message: format!("HTTP {}（服务端错误）", status),
        }),
    }
}

fn response_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// 构造请求 headers；若任何字段含非法 HTTP header 字符，返回 NotConfigured。
fn build_headers(config: &crate::config::Config) -> Result<reqwest::header::HeaderMap, AuthStatus> {
    let mut headers = reqwest::header::HeaderMap::new();

    let auth_value = format!("Bearer {}", config.bearer_token)
        .parse()
        .map_err(|_| AuthStatus::NotConfigured)?;
    headers.insert("Authorization", auth_value);

    let cookie_value = format!("auth_token={}; ct0={}", config.auth_token, config.ct0)
        .parse()
        .map_err(|_| AuthStatus::NotConfigured)?;
    headers.insert("Cookie", cookie_value);

    let csrf_value = config.ct0.parse().map_err(|_| AuthStatus::NotConfigured)?;
    headers.insert("X-Csrf-Token", csrf_value);

    if !config.user_agent.is_empty() {
        let ua = config
            .user_agent
            .parse()
            .map_err(|_| AuthStatus::NotConfigured)?;
        headers.insert("User-Agent", ua);
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config_with_bearer(bearer: &str) -> crate::config::Config {
        crate::config::Config {
            user_id: "1".into(),
            bearer_token: bearer.into(),
            auth_token: "valid_auth_token".into(),
            ct0: "valid_ct0".into(),
            personalization_id: "".into(),
            user_agent: "Mozilla/5.0".into(),
            x_client_uuid: "".into(),
            x_client_transaction_id: "".into(),
            count: "20".into(),
            all: false,
            download_dir: "data/downloads".into(),
            download_record: "data/x.txt".into(),
            file_format: "{ID}".into(),
            download_sandbox_base_dir: None,
            auto_organize: false,
            target_dir: "data/organized".into(),
            likes_api_url: "https://x.com/Likes".into(),
            likes_features: "{}".into(),
            likes_fieldtoggles: "{}".into(),
            tweet_detail_api_url: "https://x.com/TweetDetail".into(),
            tweet_features: "{}".into(),
            tweet_fieldtoggles: "{}".into(),
            mock_mode: false,
            mock_liked_tweets_file: "".into(),
        }
    }

    #[test]
    fn build_headers_valid_returns_ok() {
        let cfg = make_config_with_bearer("AAAAAAAA");
        let result = build_headers(&cfg);
        assert!(result.is_ok());
    }

    #[test]
    fn build_headers_with_newline_in_bearer_returns_not_configured() {
        // 换行符是非法 HTTP header 字符
        let cfg = make_config_with_bearer("AAA\nBBB");
        let result = build_headers(&cfg);
        assert!(matches!(result, Err(AuthStatus::NotConfigured)));
    }

    #[test]
    fn build_headers_with_control_char_in_bearer_returns_not_configured() {
        let cfg = make_config_with_bearer("AAA\x00BBB");
        let result = build_headers(&cfg);
        assert!(matches!(result, Err(AuthStatus::NotConfigured)));
    }

    #[test]
    fn classify_500_is_network_error_not_endpoint_stale() {
        let s = classify_auth_response_status(500, None).unwrap();
        match s {
            AuthStatus::NetworkError { .. } => {}
            other => panic!("expected NetworkError for 500, got {:?}", other),
        }
    }

    #[test]
    fn classify_503_is_network_error_not_endpoint_stale() {
        let s = classify_auth_response_status(503, None).unwrap();
        assert!(matches!(s, AuthStatus::NetworkError { .. }));
    }

    #[test]
    fn classify_401_is_auth_expired() {
        let s = classify_auth_response_status(401, None).unwrap();
        assert!(matches!(s, AuthStatus::AuthExpired));
    }

    #[test]
    fn classify_404_is_endpoint_stale() {
        let s = classify_auth_response_status(404, None).unwrap();
        assert!(matches!(s, AuthStatus::EndpointStale));
    }

    #[test]
    fn classify_429_is_rate_limited_with_retry_after() {
        let s = classify_auth_response_status(429, Some(60)).unwrap();
        match s {
            AuthStatus::RateLimited { retry_after } => assert_eq!(retry_after, Some(60)),
            other => panic!("expected RateLimited, got {:?}", other),
        }
    }

    #[test]
    fn classify_200_is_none_meaning_check_body() {
        let s = classify_auth_response_status(200, None);
        assert!(s.is_none(), "200 needs body inspection by caller");
    }
}
