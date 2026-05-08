use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Vendored protocol defaults — embedded at compile time from `skill/defaults.json`.
/// These act as out-of-the-box fallbacks; users override via `xld setup` or env vars.
const VENDORED_DEFAULTS: &str = include_str!("../skill/defaults.json");

/// 返回凭据文件的稳定路径，按以下优先级解析：
///
/// 1. `XLD_CREDENTIALS_FILE` 环境变量
/// 2. `./data/private_tokens.env`（若已存在——legacy 兼容）
/// 3. 平台标准用户数据目录下的 `xld/private_tokens.env`
///    - macOS: `~/Library/Application Support/xld/private_tokens.env`
///    - Linux: `${XDG_DATA_HOME:-~/.local/share}/xld/private_tokens.env`
///    - Windows: `%LOCALAPPDATA%\xld\private_tokens.env`
/// 4. fallback 到当前目录 `./data/private_tokens.env`（dirs 不可用时）
pub fn credentials_path() -> PathBuf {
    // 在解析路径前确保 .env 已加载——dotenv 是 idempotent 的，
    // 不会覆盖已设置的环境变量，多次调用安全。这让 `xld setup`
    // 与后续命令对 `XLD_CREDENTIALS_FILE`（通常写在 .env）的解析一致。
    let _ = dotenv::dotenv();

    if let Ok(p) = env::var("XLD_CREDENTIALS_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let legacy = PathBuf::from("data/private_tokens.env");
    if legacy.exists() {
        return legacy;
    }
    if let Some(data_dir) = dirs::data_dir() {
        return data_dir.join("xld").join("private_tokens.env");
    }
    legacy
}

/// Hardcoded fallback for X Web's public bearer token. Used only when neither
/// env var, private_tokens.env, nor skill/defaults.json supply one.
const HARDCODED_BEARER: &str =
    "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3DbYqd8UMSvvy";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // 用户认证信息
    pub user_id: String,
    pub bearer_token: String,
    pub auth_token: String,
    pub ct0: String,
    pub personalization_id: String,
    pub user_agent: String,
    pub x_client_uuid: String,
    pub x_client_transaction_id: String,

    // 下载配置
    pub count: String,
    pub all: bool,
    pub download_dir: String,
    pub download_record: String,
    pub file_format: String,

    // 沙箱配置（Agent 模式下使用）
    pub download_sandbox_base_dir: Option<PathBuf>,

    // 整理配置
    pub auto_organize: bool,
    pub target_dir: String,

    // API配置
    pub likes_api_url: String,
    pub likes_features: String,
    pub likes_fieldtoggles: String,
    pub tweet_detail_api_url: String,
    pub tweet_features: String,
    pub tweet_fieldtoggles: String,

    // Mock配置
    pub mock_mode: bool,
    pub mock_liked_tweets_file: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenv::dotenv().ok();

        let private_tokens = Self::load_private_tokens(&credentials_path())?;
        let defaults: Value = serde_json::from_str(VENDORED_DEFAULTS)
            .context("解析 vendored skill/defaults.json 失败")?;

        Ok(Config {
            // 私密字段：来源固定为 private_tokens（不走三层）
            user_id: private_tokens
                .get("USER_ID")
                .cloned()
                .unwrap_or_default(),
            bearer_token: resolve_protocol_field(
                "BEARER_TOKEN",
                "bearer_token",
                &private_tokens,
                &defaults,
                HARDCODED_BEARER,
            ),
            auth_token: private_tokens
                .get("AUTH_TOKEN")
                .cloned()
                .unwrap_or_default(),
            ct0: private_tokens.get("CT0").cloned().unwrap_or_default(),
            personalization_id: private_tokens
                .get("PERSONALIZATION_ID")
                .cloned()
                .unwrap_or_default(),
            user_agent: private_tokens
                .get("USER_AGENT")
                .cloned()
                .unwrap_or_default(),
            x_client_uuid: private_tokens
                .get("X_CLIENT_UUID")
                .cloned()
                .unwrap_or_default(),
            x_client_transaction_id: private_tokens
                .get("X_CLIENT_TRANSACTION_ID")
                .cloned()
                .unwrap_or_default(),

            // 下载/整理：env-only（保留旧行为）
            count: env::var("COUNT").unwrap_or_else(|_| "20".to_string()),
            all: env::var("ALL")
                .unwrap_or_else(|_| "False".to_string())
                .to_lowercase()
                == "true",
            download_dir: env::var("DOWNLOAD_DIR")
                .unwrap_or_else(|_| "data/downloads".to_string()),
            download_record: env::var("DOWNLOAD_RECORD")
                .unwrap_or_else(|_| "data/downloaded_tweet_ids.txt".to_string()),
            file_format: env::var("FILE_FORMAT")
                .unwrap_or_else(|_| "{USERNAME} {ID}".to_string()),
            download_sandbox_base_dir: resolve_sandbox_base_dir(&private_tokens),
            auto_organize: env::var("AUTO_ORGANIZE")
                .unwrap_or_else(|_| "False".to_string())
                .to_lowercase()
                == "true",
            target_dir: env::var("TARGET_DIR")
                .unwrap_or_else(|_| "data/organized".to_string()),

            // 协议字段：env > private_tokens > defaults.json > hardcoded
            likes_api_url: resolve_protocol_field(
                "LIKES_API_URL",
                "likes_api_url",
                &private_tokens,
                &defaults,
                "https://x.com/i/api/graphql/nWpDa3j6UoobbTNcFu_Uog/Likes",
            ),
            likes_features: resolve_protocol_field(
                "LIKES_FEATURES",
                "likes_features",
                &private_tokens,
                &defaults,
                r#"{}"#,
            ),
            likes_fieldtoggles: resolve_protocol_field(
                "LIKES_FIELDTOGGLES",
                "likes_fieldtoggles",
                &private_tokens,
                &defaults,
                r#"{"withArticlePlainText":false}"#,
            ),
            tweet_detail_api_url: env::var("TWEET_DETAIL_API_URL").unwrap_or_else(|_| "https://x.com/i/api/graphql/_8aYOgEDz35BrBcBal1-_w/TweetDetail".to_string()),
            tweet_features: env::var("TWEET_FEATURES").unwrap_or_else(|_| r#"{"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":false,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_show_grok_translated_post":false,"responsive_web_grok_analysis_button_from_backend":false,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_enhance_cards_enabled":false}"#.to_string()),
            tweet_fieldtoggles: env::var("TWEET_FIELDTOGGLES").unwrap_or_else(|_| r#"{"withArticleRichContentState":true,"withArticlePlainText":false,"withGrokAnalyze":false,"withDisallowedReplyControls":false}"#.to_string()),
            mock_mode: env::var("MOCK_MODE").unwrap_or_else(|_| "False".to_string()).to_lowercase() == "true",
            mock_liked_tweets_file: env::var("MOCK_LIKED_TWEETS_FILE").unwrap_or_else(|_| "data/mock/mock_liked_tweets.json".to_string()),
        })
    }

    fn load_private_tokens(path: &Path) -> Result<HashMap<String, String>> {
        // For Agent / skill workflows the file may legitimately not exist yet
        // (e.g. before `xld setup`); return an empty map and let downstream
        // code decide whether the missing fields are fatal.
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("无法读取文件: {}", path.display()))?;

        let mut tokens = HashMap::new();
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                tokens.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(tokens)
    }

    /// Whether all critical auth fields are populated. Used by `auth_status`
    /// to short-circuit to `not_configured` without a network round-trip.
    pub fn is_configured(&self) -> bool {
        !self.user_id.is_empty()
            && !self.auth_token.is_empty()
            && !self.ct0.is_empty()
            && !self.bearer_token.is_empty()
            && !self.likes_api_url.is_empty()
    }
}

/// Three-layer protocol field resolution.
///
/// Priority (high → low):
///   1. env::var (driven by .env or actual env)
///   2. private_tokens.env (written by `xld setup` from cURL)
///   3. skill/defaults.json (vendored fallback)
///   4. hardcoded fallback
fn resolve_protocol_field(
    env_key: &str,
    json_key: &str,
    private_tokens: &HashMap<String, String>,
    defaults: &Value,
    hardcoded: &str,
) -> String {
    if let Ok(v) = env::var(env_key) {
        if !v.is_empty() {
            return v;
        }
    }
    if let Some(v) = private_tokens.get(env_key) {
        if !v.is_empty() {
            return v.clone();
        }
    }
    if let Some(v) = defaults.get(json_key).and_then(|x| x.as_str()) {
        return v.to_string();
    }
    hardcoded.to_string()
}

fn resolve_sandbox_base_dir(private_tokens: &HashMap<String, String>) -> Option<PathBuf> {
    if let Ok(v) = env::var("DOWNLOAD_SANDBOX_BASE_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    if let Some(v) = private_tokens.get("DOWNLOAD_SANDBOX_BASE_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn vendored_defaults_parses() {
        let v: Value = serde_json::from_str(VENDORED_DEFAULTS).unwrap();
        assert!(v.get("likes_api_url").is_some());
        assert_eq!(v.get("schema_version").and_then(|x| x.as_u64()), Some(1));
    }

    #[test]
    fn vendored_defaults_has_no_secrets() {
        let v: Value = serde_json::from_str(VENDORED_DEFAULTS).unwrap();
        let obj = v.as_object().unwrap();
        for forbidden in &[
            "auth_token",
            "ct0",
            "user_id",
            "user_agent",
            "personalization_id",
        ] {
            assert!(
                !obj.contains_key(*forbidden),
                "skill/defaults.json 不允许包含敏感字段 {}",
                forbidden
            );
        }
    }

    #[test]
    fn private_tokens_overrides_defaults() {
        let mut pt = HashMap::new();
        pt.insert("LIKES_API_URL".into(), "https://example.com/Likes".into());
        let defaults: Value = serde_json::from_str(VENDORED_DEFAULTS).unwrap();

        let resolved =
            resolve_protocol_field("LIKES_API_URL", "likes_api_url", &pt, &defaults, "fallback");
        assert_eq!(resolved, "https://example.com/Likes");
    }

    #[test]
    fn defaults_used_when_private_tokens_empty() {
        let pt = HashMap::new();
        let defaults: Value = serde_json::from_str(VENDORED_DEFAULTS).unwrap();

        let resolved =
            resolve_protocol_field("LIKES_API_URL", "likes_api_url", &pt, &defaults, "fallback");
        assert!(resolved.starts_with("https://x.com/i/api/graphql/"));
        assert!(resolved.ends_with("/Likes"));
    }

    #[test]
    fn hardcoded_used_when_all_layers_empty() {
        let pt = HashMap::new();
        let defaults = serde_json::json!({});
        let resolved = resolve_protocol_field("MISSING_KEY", "missing", &pt, &defaults, "fallback");
        assert_eq!(resolved, "fallback");
    }

    #[test]
    fn credentials_path_respects_env_override() {
        use std::env;
        // 用一个唯一名避免和系统其它测试冲突
        let key = "XLD_CREDENTIALS_FILE";
        let saved = env::var(key).ok();
        env::set_var(key, "/tmp/xld-test-cred-override.env");
        let p = credentials_path();
        assert_eq!(
            p,
            std::path::PathBuf::from("/tmp/xld-test-cred-override.env")
        );
        // 还原
        match saved {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }

    #[test]
    fn is_configured_requires_user_id() {
        let mut cfg = Config {
            user_id: "1".into(),
            bearer_token: "b".into(),
            auth_token: "a".into(),
            ct0: "c".into(),
            personalization_id: "".into(),
            user_agent: "".into(),
            x_client_uuid: "".into(),
            x_client_transaction_id: "".into(),
            count: "20".into(),
            all: false,
            download_dir: "".into(),
            download_record: "".into(),
            file_format: "".into(),
            download_sandbox_base_dir: None,
            auto_organize: false,
            target_dir: "".into(),
            likes_api_url: "https://x.com/Likes".into(),
            likes_features: "{}".into(),
            likes_fieldtoggles: "{}".into(),
            tweet_detail_api_url: "".into(),
            tweet_features: "".into(),
            tweet_fieldtoggles: "".into(),
            mock_mode: false,
            mock_liked_tweets_file: "".into(),
        };
        assert!(cfg.is_configured(), "完整字段应当 configured");

        cfg.user_id = String::new();
        assert!(!cfg.is_configured(), "user_id 缺失应当 not configured");
    }

    #[test]
    fn credentials_path_returns_pathbuf() {
        // 不依赖文件系统状态——只断言返回的 PathBuf 末段含 "private_tokens.env"
        let saved = std::env::var("XLD_CREDENTIALS_FILE").ok();
        std::env::remove_var("XLD_CREDENTIALS_FILE");
        let p = credentials_path();
        let s = p.to_string_lossy();
        assert!(s.ends_with("private_tokens.env"), "got: {}", s);
        if let Some(v) = saved {
            std::env::set_var("XLD_CREDENTIALS_FILE", v);
        }
    }
}
