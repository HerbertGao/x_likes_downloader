use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

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

        let private_tokens = Self::load_private_tokens("data/private_tokens.env")?;

        Ok(Config {
            // 从private_tokens加载
            user_id: private_tokens.get("USER_ID").unwrap_or(&"".to_string()).clone(),
            bearer_token: private_tokens.get("BEARER_TOKEN").unwrap_or(&"".to_string()).clone(),
            auth_token: private_tokens.get("AUTH_TOKEN").unwrap_or(&"".to_string()).clone(),
            ct0: private_tokens.get("CT0").unwrap_or(&"".to_string()).clone(),
            personalization_id: private_tokens.get("PERSONALIZATION_ID").unwrap_or(&"".to_string()).clone(),
            user_agent: private_tokens.get("USER_AGENT").unwrap_or(&"".to_string()).clone(),
            x_client_uuid: private_tokens.get("X_CLIENT_UUID").unwrap_or(&"".to_string()).clone(),
            x_client_transaction_id: private_tokens.get("X_CLIENT_TRANSACTION_ID").unwrap_or(&"".to_string()).clone(),

            // 从环境变量加载
            count: env::var("COUNT").unwrap_or_else(|_| "20".to_string()),
            all: env::var("ALL").unwrap_or_else(|_| "False".to_string()).to_lowercase() == "true",
            download_dir: env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "data/downloads".to_string()),
            download_record: env::var("DOWNLOAD_RECORD").unwrap_or_else(|_| "data/downloaded_tweet_ids.txt".to_string()),
            file_format: env::var("FILE_FORMAT").unwrap_or_else(|_| "{USERNAME} {ID}".to_string()),
            auto_organize: env::var("AUTO_ORGANIZE").unwrap_or_else(|_| "False".to_string()).to_lowercase() == "true",
            target_dir: env::var("TARGET_DIR").unwrap_or_else(|_| "data/organized".to_string()),
            likes_api_url: env::var("LIKES_API_URL").unwrap_or_else(|_| "https://x.com/i/api/graphql/nWpDa3j6UoobbTNcFu_Uog/Likes".to_string()),
            likes_features: env::var("LIKES_FEATURES").unwrap_or_else(|_| r#"{"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"responsive_web_graphql_exclude_directive_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":false,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_analysis_button_from_backend":false,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"rweb_video_timestamps_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":false,"responsive_web_enhance_cards_enabled":false}"#.to_string()),
            likes_fieldtoggles: env::var("LIKES_FIELDTOGGLES").unwrap_or_else(|_| r#"{"withArticlePlainText":false}"#.to_string()),
            tweet_detail_api_url: env::var("TWEET_DETAIL_API_URL").unwrap_or_else(|_| "https://x.com/i/api/graphql/_8aYOgEDz35BrBcBal1-_w/TweetDetail".to_string()),
            tweet_features: env::var("TWEET_FEATURES").unwrap_or_else(|_| r#"{"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":false,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_show_grok_translated_post":false,"responsive_web_grok_analysis_button_from_backend":false,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_enhance_cards_enabled":false}"#.to_string()),
            tweet_fieldtoggles: env::var("TWEET_FIELDTOGGLES").unwrap_or_else(|_| r#"{"withArticleRichContentState":true,"withArticlePlainText":false,"withGrokAnalyze":false,"withDisallowedReplyControls":false}"#.to_string()),
            mock_mode: env::var("MOCK_MODE").unwrap_or_else(|_| "False".to_string()).to_lowercase() == "true",
            mock_liked_tweets_file: env::var("MOCK_LIKED_TWEETS_FILE").unwrap_or_else(|_| "data/mock/mock_liked_tweets.json".to_string()),
        })
    }

    fn load_private_tokens(filename: &str) -> Result<HashMap<String, String>> {
        if !Path::new(filename).exists() {
            return Err(anyhow::anyhow!("{} 不存在，请先运行 setup 命令初始化。", filename));
        }

        let content = fs::read_to_string(filename)
            .with_context(|| format!("无法读取文件: {}", filename))?;

        let mut tokens = HashMap::new();
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                tokens.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(tokens)
    }
} 