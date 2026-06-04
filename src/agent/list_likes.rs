use serde_json::{json, Value};
use std::time::Duration;

use crate::config::Config;
use crate::error::{classify_status, ErrorKind, ErrorPayload};

use super::types::{ListOpts, ListOutput, MediaItem, MediaType, TweetSummary};

const SCHEMA_VERSION: u32 = 1;

pub async fn list_likes(opts: ListOpts) -> Result<ListOutput, ErrorPayload> {
    let config = Config::load()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, format!("加载配置失败: {}", e)))?;

    if !config.is_configured() {
        return Err(ErrorPayload::new(
            ErrorKind::NotConfigured,
            "本地未导入凭据",
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    let count = opts
        .count
        .unwrap_or_else(|| config.count.parse::<u32>().unwrap_or(20));

    let mut all_summaries: Vec<TweetSummary> = Vec::new();
    let mut all_raw: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = opts.since_cursor.clone();

    loop {
        let (entries, new_cursor) =
            fetch_one_page(&client, &config, count, cursor.as_deref()).await?;

        for entry in entries {
            if let Some(summary) = entry_to_summary(&entry, opts.include_raw) {
                all_summaries.push(summary);
                if opts.include_raw {
                    all_raw.push(entry);
                }
            }
        }

        if !opts.all {
            cursor = new_cursor;
            break;
        }

        match new_cursor {
            None => {
                cursor = None;
                break;
            }
            Some(ref c) if Some(c) == cursor.as_ref() => {
                cursor = new_cursor;
                break;
            }
            _ => {
                cursor = new_cursor;
            }
        }
    }

    Ok(ListOutput {
        tweets: all_summaries,
        cursor,
        schema_version: SCHEMA_VERSION,
        raw_entries: if opts.include_raw {
            Some(all_raw)
        } else {
            None
        },
    })
}

async fn fetch_one_page(
    client: &reqwest::Client,
    config: &Config,
    count: u32,
    cursor: Option<&str>,
) -> Result<(Vec<Value>, Option<String>), ErrorPayload> {
    let mut variables = json!({
        "userId": config.user_id,
        "count": count as i64,
        "includePromotedContent": false,
        "withClientEventToken": false,
        "withBirdwatchNotes": false,
        "withVoice": true,
        "withV2Timeline": true
    });
    if let Some(c) = cursor {
        variables["cursor"] = json!(c);
    }

    let variables_str = serde_json::to_string(&variables)
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;
    let variables_encoded = urlencoding::encode(&variables_str);
    let features_encoded = urlencoding::encode(&config.likes_features);
    let fieldtoggles_encoded = urlencoding::encode(&config.likes_fieldtoggles);

    let url = format!(
        "{}?variables={}&features={}&fieldToggles={}",
        config.likes_api_url, variables_encoded, features_encoded, fieldtoggles_encoded
    );

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Authorization",
        format!("Bearer {}", config.bearer_token).parse().map_err(
            |e: reqwest::header::InvalidHeaderValue| {
                ErrorPayload::new(ErrorKind::InternalError, e.to_string())
            },
        )?,
    );
    headers.insert(
        "Cookie",
        format!("auth_token={}; ct0={}", config.auth_token, config.ct0)
            .parse()
            .map_err(|e: reqwest::header::InvalidHeaderValue| {
                ErrorPayload::new(ErrorKind::InternalError, e.to_string())
            })?,
    );
    headers.insert(
        "X-Csrf-Token",
        config
            .ct0
            .parse()
            .map_err(|e: reqwest::header::InvalidHeaderValue| {
                ErrorPayload::new(ErrorKind::InternalError, e.to_string())
            })?,
    );
    if !config.user_agent.is_empty() {
        headers.insert(
            "User-Agent",
            config
                .user_agent
                .parse()
                .map_err(|e: reqwest::header::InvalidHeaderValue| {
                    ErrorPayload::new(ErrorKind::InternalError, e.to_string())
                })?,
        );
    }

    let response = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| ErrorPayload::new(ErrorKind::NetworkError, e.to_string()))?;

    let status = response.status().as_u16();
    if let Some(kind) = classify_status(status) {
        let body = response.text().await.unwrap_or_default();
        let mut payload = ErrorPayload::new(
            kind,
            format!(
                "HTTP {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            ),
        );
        if kind == ErrorKind::RateLimited {
            payload = payload.with_retry_after(60);
        }
        return Err(payload);
    }

    let data: Value = response.json().await.map_err(|e| {
        ErrorPayload::new(
            ErrorKind::InternalError,
            format!("解析响应 JSON 失败: {}", e),
        )
    })?;

    Ok(parse_likes_response(&data))
}

/// 在 `data.user.result` 下找外层 timeline 节点。
///
/// X 历史上把它叫 `timeline_v2`（V2 timeline 是 opt-in feature），
/// 在 V2 timeline 全量普及后字段改名为 `timeline`。两种字段名都要兼容：
/// 优先 `timeline_v2`（保留对老响应的解析能力），回退 `timeline`（当前默认）。
fn find_outer_timeline(data: &Value) -> Option<&Value> {
    let result = data
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("result"))?;
    result.get("timeline_v2").or_else(|| result.get("timeline"))
}

/// Parse the GraphQL Likes response, returning (tweet entries, next cursor).
pub(crate) fn parse_likes_response(data: &Value) -> (Vec<Value>, Option<String>) {
    let mut tweets = Vec::new();
    let mut new_cursor = None;

    if let Some(instructions) = find_outer_timeline(data)
        .and_then(|t| t.get("timeline"))
        .and_then(|t| t.get("instructions"))
        .and_then(|i| i.as_array())
    {
        for instruction in instructions {
            if instruction.get("type") == Some(&json!("TimelineAddEntries")) {
                if let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) {
                    for entry in entries {
                        if let Some(entry_id) = entry.get("entryId").and_then(|id| id.as_str()) {
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

    (tweets, new_cursor)
}

/// Convert a single `tweet-*` entry from X GraphQL into a flat TweetSummary.
pub(crate) fn entry_to_summary(entry: &Value, include_raw: bool) -> Option<TweetSummary> {
    let tweet_result = entry
        .get("content")
        .and_then(|c| c.get("itemContent"))
        .and_then(|ic| ic.get("tweet_results"))
        .and_then(|tr| tr.get("result"))?;

    // Sometimes wrapped in an extra "tweet" object (TweetWithVisibilityResults)
    let tweet_obj = tweet_result.get("tweet").unwrap_or(tweet_result);

    let id = tweet_obj
        .get("rest_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;

    let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);

    // X 在 schema 演进中把 user 的 `screen_name` / `name` 从 `user.legacy` 挪到了 `user.core`。
    // 两个位置都接受：优先新位置 `core`，回退 `legacy`，再回退顶层（极端兜底）。
    let user_result = tweet_obj
        .get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|ur| ur.get("result"));

    let lookup_user_field = |field: &str| -> String {
        user_result
            .and_then(|r| r.get("core"))
            .and_then(|c| c.get(field))
            .and_then(|s| s.as_str())
            .or_else(|| {
                user_result
                    .and_then(|r| r.get("legacy"))
                    .and_then(|l| l.get(field))
                    .and_then(|s| s.as_str())
            })
            .unwrap_or("")
            .to_string()
    };

    let author_handle = lookup_user_field("screen_name");
    let author_display_name = lookup_user_field("name");

    let text = legacy
        .get("full_text")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let created_at = legacy
        .get("created_at")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();

    let is_retweet = legacy.get("retweeted_status_result").is_some()
        || legacy
            .get("retweeted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let is_reply = legacy
        .get("in_reply_to_status_id_str")
        .and_then(|v| v.as_str())
        .is_some();

    let tweet_url = if !author_handle.is_empty() {
        format!("https://x.com/{}/status/{}", author_handle, id)
    } else {
        format!("https://x.com/i/status/{}", id)
    };

    let media = extract_media(&id, &author_handle, &created_at, legacy, include_raw);

    Some(TweetSummary {
        id,
        author_handle,
        author_display_name,
        text,
        created_at,
        tweet_url,
        is_retweet,
        is_reply,
        media,
        liked_at: None,
    })
}

pub(crate) fn extract_media(
    tweet_id: &str,
    author_handle: &str,
    tweet_created_at: &str,
    legacy: &Value,
    include_raw: bool,
) -> Vec<MediaItem> {
    let handle_opt = if author_handle.is_empty() {
        None
    } else {
        Some(author_handle.to_string())
    };
    let created_opt = if tweet_created_at.is_empty() {
        None
    } else {
        Some(tweet_created_at.to_string())
    };

    let mut out = Vec::new();
    let media_array = legacy
        .get("extended_entities")
        .and_then(|ee| ee.get("media"))
        .and_then(|m| m.as_array())
        .or_else(|| {
            legacy
                .get("entities")
                .and_then(|e| e.get("media"))
                .and_then(|m| m.as_array())
        });

    let Some(media_array) = media_array else {
        return out;
    };

    for media in media_array {
        let media_type = media.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match media_type {
            "video" | "animated_gif" => {
                let kind = if media_type == "video" {
                    MediaType::Video
                } else {
                    MediaType::Gif
                };
                if let Some(variants) = media
                    .get("video_info")
                    .and_then(|v| v.get("variants"))
                    .and_then(|v| v.as_array())
                {
                    let best = if media_type == "animated_gif" {
                        // GIF 通常只有一个 mp4 variant，且常常无 bitrate 字段；不要求 bitrate
                        variants.iter().find(|v| {
                            v.get("content_type")
                                .and_then(|s| s.as_str())
                                .map(|s| s == "video/mp4")
                                .unwrap_or(false)
                        })
                    } else {
                        // Video: 选 bitrate 最高的 mp4 variant；保留 bitrate 必填以避免选到 m3u8 占位
                        variants
                            .iter()
                            .filter(|v| {
                                v.get("content_type")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s == "video/mp4")
                                    .unwrap_or(false)
                                    && v.get("bitrate").is_some()
                            })
                            .max_by_key(|v| v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0))
                    };
                    if let Some(variant) = best {
                        if let Some(url) = variant.get("url").and_then(|u| u.as_str()) {
                            let suggested = derive_filename(url);
                            out.push(MediaItem {
                                tweet_id: tweet_id.to_string(),
                                kind: kind.clone(),
                                url: url.to_string(),
                                suggested_filename: suggested,
                                bytes: None,
                                author_handle: handle_opt.clone(),
                                created_at: created_opt.clone(),
                                all_variants: if include_raw {
                                    Some(variants.clone())
                                } else {
                                    None
                                },
                            });
                        }
                    }
                }
            }
            "photo" => {
                if let Some(base) = media.get("media_url_https").and_then(|u| u.as_str()) {
                    let url = if base.contains('?') {
                        base.to_string()
                    } else {
                        format!("{}?format=jpg&name=orig", base)
                    };
                    let suggested = derive_filename(&url);
                    out.push(MediaItem {
                        tweet_id: tweet_id.to_string(),
                        kind: MediaType::Image,
                        url,
                        suggested_filename: suggested,
                        bytes: None,
                        author_handle: handle_opt.clone(),
                        created_at: created_opt.clone(),
                        all_variants: None,
                    });
                }
            }
            _ => {}
        }
    }

    out
}

fn derive_filename(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(seg) = parsed
            .path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|s| !s.is_empty())
        {
            return seg.to_string();
        }
    }
    "media".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_filename_strips_path() {
        assert_eq!(
            derive_filename("https://pbs.twimg.com/media/AAA.jpg?format=jpg&name=orig"),
            "AAA.jpg"
        );
    }

    #[test]
    fn entry_to_summary_basic() {
        let entry = json!({
            "entryId": "tweet-1234",
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "rest_id": "1234",
                            "core": {
                                "user_results": {
                                    "result": {
                                        "legacy": {
                                            "screen_name": "alice",
                                            "name": "Alice"
                                        }
                                    }
                                }
                            },
                            "legacy": {
                                "full_text": "hello",
                                "created_at": "Thu Apr 06 15:24:15 +0000 2017"
                            }
                        }
                    }
                }
            }
        });
        let s = entry_to_summary(&entry, false).unwrap();
        assert_eq!(s.id, "1234");
        assert_eq!(s.author_handle, "alice");
        assert_eq!(s.author_display_name, "Alice");
        assert_eq!(s.text, "hello");
        assert_eq!(s.tweet_url, "https://x.com/alice/status/1234");
        assert!(s.media.is_empty());
    }

    #[test]
    fn extract_media_picks_highest_bitrate_mp4() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "video",
                    "video_info": {
                        "variants": [
                            { "content_type": "application/x-mpegURL", "url": "https://x.tv/foo.m3u8" },
                            { "content_type": "video/mp4", "bitrate": 320000, "url": "https://x.tv/lo.mp4" },
                            { "content_type": "video/mp4", "bitrate": 2176000, "url": "https://x.tv/hi.mp4" }
                        ]
                    }
                }]
            }
        });
        let media = extract_media(
            "123",
            "alice",
            "Thu Apr 06 15:24:15 +0000 2017",
            &legacy,
            false,
        );
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].kind, MediaType::Video);
        assert_eq!(media[0].url, "https://x.tv/hi.mp4");
        assert_eq!(media[0].suggested_filename, "hi.mp4");
    }

    #[test]
    fn extract_media_image_uses_orig_size() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "photo",
                    "media_url_https": "https://pbs.twimg.com/media/ABC.jpg"
                }]
            }
        });
        let media = extract_media("999", "bob", "", &legacy, false);
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].kind, MediaType::Image);
        assert!(media[0].url.contains("name=orig"));
        assert_eq!(media[0].author_handle.as_deref(), Some("bob"));
        assert_eq!(media[0].created_at, None);
    }

    #[test]
    fn extract_media_gif_without_bitrate() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "animated_gif",
                    "video_info": {
                        "variants": [
                            // X 实际响应常态：无 bitrate
                            { "content_type": "video/mp4", "url": "https://x.tv/no-bitrate.mp4" }
                        ]
                    }
                }]
            }
        });
        let media = extract_media("g2", "alice", "", &legacy, false);
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].kind, MediaType::Gif);
        assert_eq!(media[0].url, "https://x.tv/no-bitrate.mp4");
    }

    #[test]
    fn extract_media_animated_gif() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "animated_gif",
                    "video_info": {
                        "variants": [
                            { "content_type": "video/mp4", "bitrate": 0, "url": "https://x.tv/gif.mp4" }
                        ]
                    }
                }]
            }
        });
        let media = extract_media(
            "g1",
            "carol",
            "Thu Apr 06 00:00:00 +0000 2020",
            &legacy,
            false,
        );
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].kind, MediaType::Gif);
        assert_eq!(media[0].author_handle.as_deref(), Some("carol"));
        assert_eq!(
            media[0].created_at.as_deref(),
            Some("Thu Apr 06 00:00:00 +0000 2020")
        );
    }

    #[test]
    fn extract_media_omits_all_variants_when_include_raw_false() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "video",
                    "video_info": {
                        "variants": [
                            { "content_type": "video/mp4", "bitrate": 320000, "url": "https://x.tv/lo.mp4" },
                            { "content_type": "video/mp4", "bitrate": 2176000, "url": "https://x.tv/hi.mp4" }
                        ]
                    }
                }]
            }
        });
        let media = extract_media("v1", "alice", "", &legacy, false);
        assert_eq!(media.len(), 1);
        assert!(
            media[0].all_variants.is_none(),
            "default 模式不应携带 all_variants"
        );
    }

    #[test]
    fn extract_media_includes_all_variants_when_include_raw_true() {
        let legacy = json!({
            "extended_entities": {
                "media": [{
                    "type": "video",
                    "video_info": {
                        "variants": [
                            { "content_type": "video/mp4", "bitrate": 320000, "url": "https://x.tv/lo.mp4" },
                            { "content_type": "video/mp4", "bitrate": 2176000, "url": "https://x.tv/hi.mp4" }
                        ]
                    }
                }]
            }
        });
        let media = extract_media("v1", "alice", "", &legacy, true);
        assert_eq!(media.len(), 1);
        let vars = media[0]
            .all_variants
            .as_ref()
            .expect("include_raw=true 时应含 all_variants");
        assert_eq!(vars.len(), 2);
    }

    /// X 当前 schema 把外层节点叫 `timeline`（不带 _v2）。
    #[test]
    fn parse_likes_response_accepts_timeline_schema() {
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [{
                                    "type": "TimelineAddEntries",
                                    "entries": [
                                        { "entryId": "tweet-1234" },
                                        {
                                            "entryId": "cursor-bottom-x",
                                            "content": { "value": "next-cursor" }
                                        }
                                    ]
                                }]
                            }
                        }
                    }
                }
            }
        });
        let (tweets, cursor) = parse_likes_response(&data);
        assert_eq!(tweets.len(), 1);
        assert_eq!(cursor, Some("next-cursor".to_string()));
    }

    /// 旧 schema（V2 timeline opt-in 时期）把外层节点叫 `timeline_v2`，仍要兼容。
    #[test]
    fn parse_likes_response_accepts_timeline_v2_schema() {
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline_v2": {
                            "timeline": {
                                "instructions": [{
                                    "type": "TimelineAddEntries",
                                    "entries": [{ "entryId": "tweet-9999" }]
                                }]
                            }
                        }
                    }
                }
            }
        });
        let (tweets, cursor) = parse_likes_response(&data);
        assert_eq!(tweets.len(), 1);
        assert_eq!(cursor, None);
    }

    #[test]
    fn parse_likes_response_unknown_schema_returns_empty() {
        let data = json!({"data":{"user":{"result":{"unknown_field": {}}}}});
        let (tweets, cursor) = parse_likes_response(&data);
        assert!(tweets.is_empty());
        assert!(cursor.is_none());
    }

    /// X schema 演进：screen_name / name 从 user.legacy 挪到 user.core。新 schema 应识别。
    #[test]
    fn entry_to_summary_handles_new_user_core_schema() {
        let entry = json!({
            "entryId": "tweet-1",
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "rest_id": "1",
                            "core": {
                                "user_results": {
                                    "result": {
                                        "core": {
                                            "screen_name": "newhandle",
                                            "name": "New Display"
                                        }
                                    }
                                }
                            },
                            "legacy": { "full_text": "hi", "created_at": "Thu Apr 06 15:24:15 +0000 2017" }
                        }
                    }
                }
            }
        });
        let s = entry_to_summary(&entry, false).unwrap();
        assert_eq!(s.author_handle, "newhandle");
        assert_eq!(s.author_display_name, "New Display");
    }

    /// 老推文（2023）焦点 entry 的 `tweet_results.result.__typename` 为
    /// `TweetWithVisibilityResults`，真正的 tweet 嵌在 `result.tweet`。
    /// 验证 `entry_to_summary` 的 wrapper 解包能取到非空 legacy + media。
    /// fixture 是完整 TweetDetail 响应，焦点 entry 在
    /// `data.threaded_conversation_with_injections_v2.instructions[].entries[]`
    /// 里 entryId 形如 `tweet-<id>`。
    #[test]
    fn entry_to_summary_unwraps_tweet_with_visibility_results() {
        const FIXTURE: &str = include_str!(
            "../../tests/fixtures/tweet_detail/old_2023_TweetWithVisibilityResults.json"
        );
        let resp: Value = serde_json::from_str(FIXTURE).unwrap();

        let focal_entry = resp
            .get("data")
            .and_then(|d| d.get("threaded_conversation_with_injections_v2"))
            .and_then(|t| t.get("instructions"))
            .and_then(|i| i.as_array())
            .and_then(|instructions| {
                instructions.iter().find_map(|inst| {
                    inst.get("entries")
                        .and_then(|e| e.as_array())
                        .and_then(|entries| {
                            entries.iter().find(|e| {
                                e.get("entryId")
                                    .and_then(|id| id.as_str())
                                    .map(|id| id.starts_with("tweet-"))
                                    .unwrap_or(false)
                            })
                        })
                })
            })
            .expect("fixture 应含一个 tweet-<id> 焦点 entry");

        // Confirm the fixture really exercises the wrapper path.
        let typename = focal_entry
            .get("content")
            .and_then(|c| c.get("itemContent"))
            .and_then(|ic| ic.get("tweet_results"))
            .and_then(|tr| tr.get("result"))
            .and_then(|r| r.get("__typename"))
            .and_then(|t| t.as_str());
        assert_eq!(typename, Some("TweetWithVisibilityResults"));

        let summary = entry_to_summary(focal_entry, false)
            .expect("wrapper 焦点 entry 应能解析为 TweetSummary");
        assert!(!summary.id.is_empty(), "解包后应取到非空 id");
        assert!(!summary.media.is_empty(), "wrapper 解包后应取到非空 media");
    }

    /// 旧 schema：screen_name / name 在 user.legacy 下，仍要识别。
    #[test]
    fn entry_to_summary_handles_legacy_user_schema() {
        let entry = json!({
            "entryId": "tweet-2",
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "rest_id": "2",
                            "core": {
                                "user_results": {
                                    "result": {
                                        "legacy": {
                                            "screen_name": "oldhandle",
                                            "name": "Old Display"
                                        }
                                    }
                                }
                            },
                            "legacy": { "full_text": "hi" }
                        }
                    }
                }
            }
        });
        let s = entry_to_summary(&entry, false).unwrap();
        assert_eq!(s.author_handle, "oldhandle");
        assert_eq!(s.author_display_name, "Old Display");
    }
}
