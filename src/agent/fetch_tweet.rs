use serde_json::Value;

use crate::config::Config;
use crate::error::{classify_status, ErrorKind, ErrorPayload};
use crate::x_api::XApi;

use super::list_likes::entry_to_summary;
use super::types::{FetchTweetOutput, FetchTweetRequest};

const SCHEMA_VERSION: u32 = 1;

/// 鉴权类 GraphQL error code（X 对失效凭据常返回 HTTP 200 + 这些 code）。
const AUTH_ERROR_CODES: [i64; 3] = [32, 64, 89];

/// 从 `FetchTweetRequest` 提取 tweet_id（纯函数）。
///
/// `id` 与 `url` 必须恰好提供其一，否则 `invalid_argument`。
/// `id` 须为纯数字；`url` 用正则提取 `/status/<digits>`（接受 x.com / twitter.com /
/// mobile.twitter.com / i/web/status / `/photo|video/N` 后缀、query、fragment、尾随斜杠）。
pub fn extract_tweet_id(req: &FetchTweetRequest) -> Result<String, ErrorPayload> {
    match (req.url.as_deref(), req.id.as_deref()) {
        (Some(_), Some(_)) => Err(ErrorPayload::new(
            ErrorKind::InvalidArgument,
            "--url 与 --id 只能提供其一",
        )),
        (None, None) => Err(ErrorPayload::new(
            ErrorKind::InvalidArgument,
            "必须提供 --url 或 --id 之一",
        )),
        (None, Some(id)) => {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
                Ok(id.to_string())
            } else {
                Err(ErrorPayload::new(
                    ErrorKind::InvalidArgument,
                    format!("--id 必须为纯数字: {}", id),
                ))
            }
        }
        (Some(url), None) => extract_id_from_url(url).ok_or_else(|| {
            ErrorPayload::new(
                ErrorKind::InvalidArgument,
                format!("无法从 URL 解析 tweet_id: {}", url),
            )
        }),
    }
}

/// 从 URL 提取 `/status/<digits>` 的数字段（`<digits>` 后遇非数字即止）。
fn extract_id_from_url(url: &str) -> Option<String> {
    // `/status/` 后紧跟一个或多个数字，后续非数字（`/photo/1`、query、尾斜杠）自然截断。
    let re = regex::Regex::new(r"/status/(\d+)").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// **阶段一**错误分类（纯函数，不持有 focal_id）。
///
/// 先按 HTTP 状态码经 `classify_status` 分类；仅 HTTP 200 时再查响应体：
/// - 顶层 `errors[]` 任一 `code ∈ {32,64,89}` → `auth_expired`
/// - `data.threaded_conversation_with_injections_v2` 缺失且无鉴权 errors → `endpoint_stale`
///
/// `tweet_unavailable` **不**在此产出（它需要 focal_id，由阶段二处理）。
pub fn classify_tweet_detail(status: u16, resp: &Value) -> Option<ErrorPayload> {
    if let Some(kind) = classify_status(status) {
        let mut payload = ErrorPayload::new(kind, format!("HTTP {}", status));
        if kind == ErrorKind::RateLimited {
            payload = payload.with_retry_after(60);
        }
        return Some(payload);
    }

    // HTTP 200 起才查 body。
    let has_auth_error = resp
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|errs| {
            errs.iter().any(|err| {
                err.get("code")
                    .and_then(|c| c.as_i64())
                    .map(|code| AUTH_ERROR_CODES.contains(&code))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if has_auth_error {
        return Some(ErrorPayload::new(
            ErrorKind::AuthExpired,
            "TweetDetail 返回 HTTP 200 但含鉴权类 errors",
        ));
    }

    let has_conversation = resp
        .get("data")
        .and_then(|d| d.get("threaded_conversation_with_injections_v2"))
        .is_some();
    if !has_conversation {
        return Some(ErrorPayload::new(
            ErrorKind::EndpointStale,
            "TweetDetail 响应缺 threaded_conversation_with_injections_v2 路径",
        ));
    }

    None
}

/// **阶段二**焦点定位（纯函数）。
///
/// `resp` 为完整响应 Value，路径 `resp["data"]["threaded_conversation_with_injections_v2"]["instructions"][]`。
///
/// 主路径：遍历各 instruction 的 `entries[]`，找顶层 `entryId == "tweet-<focal_id>"` 的 entry，
/// 返回其本体（带 `content.itemContent.tweet_results.result`，对齐 `entry_to_summary` 取法）。
///
/// 软兜底（best-effort）：主路径未命中时，遍历所有 entries（含 `conversationthread-*` module 内
/// `content.items[].item`），取解包 wrapper（`result.tweet` 优先）后 `rest_id == focal_id` 的那个，
/// 并归一化成顶层 entry 形状返回，供 `entry_to_summary` 统一消费。
pub fn parse_tweet_detail_response(resp: &Value, focal_id: &str) -> Option<Value> {
    let instructions = resp
        .get("data")
        .and_then(|d| d.get("threaded_conversation_with_injections_v2"))
        .and_then(|t| t.get("instructions"))
        .and_then(|i| i.as_array())?;

    let target_entry_id = format!("tweet-{}", focal_id);

    // 主路径：顶层 entryId 精确匹配。
    for inst in instructions {
        if let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) {
            for entry in entries {
                if entry.get("entryId").and_then(|id| id.as_str()) == Some(target_entry_id.as_str())
                {
                    return Some(entry.clone());
                }
            }
        }
    }

    // 软兜底：遍历所有 entries（含 module items），按解包后 rest_id 匹配。
    for inst in instructions {
        let Some(entries) = inst.get("entries").and_then(|e| e.as_array()) else {
            continue;
        };
        for entry in entries {
            // 顶层 entry 自身。
            if let Some(result) = entry
                .get("content")
                .and_then(|c| c.get("itemContent"))
                .and_then(|ic| ic.get("tweet_results"))
                .and_then(|tr| tr.get("result"))
            {
                if unwrapped_rest_id(result) == Some(focal_id) {
                    return Some(entry.clone());
                }
            }
            // conversationthread-* module 内层 items。
            if let Some(items) = entry
                .get("content")
                .and_then(|c| c.get("items"))
                .and_then(|i| i.as_array())
            {
                for it in items {
                    if let Some(result) = it
                        .get("item")
                        .and_then(|i| i.get("itemContent"))
                        .and_then(|ic| ic.get("tweet_results"))
                        .and_then(|tr| tr.get("result"))
                    {
                        if unwrapped_rest_id(result) == Some(focal_id) {
                            // 归一化成顶层 entry 形状供 entry_to_summary 消费。
                            return Some(serde_json::json!({
                                "content": { "itemContent": { "tweet_results": { "result": result } } }
                            }));
                        }
                    }
                }
            }
        }
    }

    None
}

/// 取解包 wrapper（`result.tweet` 优先）后的 `rest_id`。
fn unwrapped_rest_id(result: &Value) -> Option<&str> {
    let obj = result.get("tweet").unwrap_or(result);
    obj.get("rest_id").and_then(|v| v.as_str())
}

/// 取焦点 entry 解包后的 `__typename`。
fn focal_typename(entry: &Value) -> Option<&str> {
    let result = entry
        .get("content")
        .and_then(|c| c.get("itemContent"))
        .and_then(|ic| ic.get("tweet_results"))
        .and_then(|tr| tr.get("result"))?;
    let obj = result.get("tweet").unwrap_or(result);
    // __typename 优先取解包后；外层 wrapper 的 __typename（如 TweetWithVisibilityResults）不算不可用。
    obj.get("__typename")
        .or_else(|| result.get("__typename"))
        .and_then(|t| t.as_str())
}

/// 检查焦点 entry 解包后是否含可用 `legacy`。
fn focal_has_legacy(entry: &Value) -> bool {
    entry
        .get("content")
        .and_then(|c| c.get("itemContent"))
        .and_then(|ic| ic.get("tweet_results"))
        .and_then(|tr| tr.get("result"))
        .map(|result| result.get("tweet").unwrap_or(result))
        .and_then(|obj| obj.get("legacy"))
        .map(|l| !l.is_null())
        .unwrap_or(false)
}

/// 抓取单条推文的媒体元数据，输出与 `list_likes` 的 `tweets[]` 元素严格同构。
pub async fn fetch_tweet(req: FetchTweetRequest) -> Result<FetchTweetOutput, ErrorPayload> {
    let id = extract_tweet_id(&req)?;

    let config = Config::load()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, format!("加载配置失败: {}", e)))?;

    if !config.is_configured() {
        return Err(ErrorPayload::new(ErrorKind::NotConfigured, "本地未导入凭据"));
    }

    let api = XApi::new(config)
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    let (status, resp) = api
        .get_tweet_detail(&id)
        .await
        .map_err(|e| ErrorPayload::new(ErrorKind::NetworkError, e.to_string()))?;

    // 阶段一：请求级分类。
    if let Some(payload) = classify_tweet_detail(status, &resp) {
        return Err(payload);
    }

    // 阶段二：焦点定位与校验。
    let entry = parse_tweet_detail_response(&resp, &id).ok_or_else(|| {
        ErrorPayload::new(
            ErrorKind::TweetUnavailable,
            format!("响应中未找到焦点推文 {}", id),
        )
    })?;

    if let Some(tn) = focal_typename(&entry) {
        if tn == "TweetUnavailable" || tn == "TweetTombstone" {
            return Err(ErrorPayload::new(
                ErrorKind::TweetUnavailable,
                format!("焦点推文不可用: {}", tn),
            ));
        }
    }

    if !focal_has_legacy(&entry) {
        return Err(ErrorPayload::new(
            ErrorKind::TweetUnavailable,
            "焦点推文缺可用 legacy",
        ));
    }

    let tweet = entry_to_summary(&entry, false).ok_or_else(|| {
        ErrorPayload::new(ErrorKind::TweetUnavailable, "焦点推文无法解析为 TweetSummary")
    })?;

    Ok(FetchTweetOutput {
        tweet,
        schema_version: SCHEMA_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NEW_2026: &str = include_str!("../../tests/fixtures/tweet_detail/new_2026_Tweet.json");
    const OLD_2023: &str =
        include_str!("../../tests/fixtures/tweet_detail/old_2023_TweetWithVisibilityResults.json");
    const OLD_2024: &str =
        include_str!("../../tests/fixtures/tweet_detail/old_2024_Tweet_multimedia.json");

    fn req(url: Option<&str>, id: Option<&str>) -> FetchTweetRequest {
        FetchTweetRequest {
            url: url.map(String::from),
            id: id.map(String::from),
        }
    }

    // ---- 8.1 extract_tweet_id ----

    #[test]
    fn extract_id_from_url_with_query() {
        let r = req(Some("https://x.com/foo/status/123456789?s=20&t=abc"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "123456789");
    }

    #[test]
    fn extract_id_from_url_trailing_slash() {
        let r = req(Some("https://x.com/foo/status/123456789/"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "123456789");
    }

    #[test]
    fn extract_id_from_twitter_dot_com() {
        let r = req(Some("https://twitter.com/foo/status/777"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "777");
        let r = req(Some("https://mobile.twitter.com/foo/status/888"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "888");
    }

    #[test]
    fn extract_id_from_i_web_status() {
        let r = req(Some("https://x.com/i/web/status/123"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "123");
    }

    #[test]
    fn extract_id_from_photo_suffix() {
        let r = req(Some("https://x.com/foo/status/123/photo/1"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "123");
        let r = req(Some("https://x.com/foo/status/456/video/2"), None);
        assert_eq!(extract_tweet_id(&r).unwrap(), "456");
    }

    #[test]
    fn extract_id_invalid_url_fails() {
        let r = req(Some("https://x.com/foo"), None);
        let err = extract_tweet_id(&r).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn extract_id_non_numeric_id_fails() {
        let err = extract_tweet_id(&req(None, Some("abc"))).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
        let err = extract_tweet_id(&req(None, Some("12a3"))).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn extract_id_pure_numeric_id_ok() {
        assert_eq!(extract_tweet_id(&req(None, Some("999"))).unwrap(), "999");
    }

    #[test]
    fn extract_id_both_provided_fails() {
        let err =
            extract_tweet_id(&req(Some("https://x.com/foo/status/1"), Some("1"))).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn extract_id_neither_provided_fails() {
        let err = extract_tweet_id(&req(None, None)).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn extract_id_empty_string_fails() {
        let err = extract_tweet_id(&FetchTweetRequest {
            url: None,
            id: Some(String::new()),
        })
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArgument);
    }

    // ---- 8.2 parse_tweet_detail_response ----

    #[test]
    fn parse_new_2026_selects_top_level_focal() {
        let resp: Value = serde_json::from_str(NEW_2026).unwrap();
        let entry = parse_tweet_detail_response(&resp, "1700000000000039996").unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, "1700000000000039996");
    }

    #[test]
    fn parse_old_2023_unwraps_wrapper_nonempty_legacy_media() {
        let resp: Value = serde_json::from_str(OLD_2023).unwrap();
        let entry = parse_tweet_detail_response(&resp, "1700000000000015554").unwrap();
        // 内层 tweet 无 __typename，回退取外层 wrapper 名；非 Tombstone/Unavailable 即可用。
        assert_eq!(focal_typename(&entry), Some("TweetWithVisibilityResults"));
        assert!(focal_has_legacy(&entry));
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, "1700000000000015554");
        assert!(!summary.media.is_empty(), "wrapper 解包后应有 media");
    }

    #[test]
    fn parse_old_2024_multimedia_two_videos() {
        let resp: Value = serde_json::from_str(OLD_2024).unwrap();
        let entry = parse_tweet_detail_response(&resp, "1700000000000002222").unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, "1700000000000002222");
        assert_eq!(summary.media.len(), 2, "应有 2 个视频媒体");
        for m in &summary.media {
            assert_eq!(m.kind, super::super::types::MediaType::Video);
        }
    }

    #[test]
    fn parse_soft_fallback_by_rest_id() {
        // 主路径无 tweet-<id>，但某 module item 解包后 rest_id 匹配。
        let resp = json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                {
                    "type": "TimelineModule",
                    "entries": [{
                        "entryId": "conversationthread-555",
                        "content": { "items": [{
                            "item": { "itemContent": { "tweet_results": { "result": {
                                "__typename": "Tweet",
                                "rest_id": "999",
                                "legacy": { "full_text": "hi" }
                            }}}}
                        }]}
                    }]
                }
            ]}}
        });
        let entry = parse_tweet_detail_response(&resp, "999").unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, "999");
    }

    #[test]
    fn parse_soft_fallback_wrapper_in_module_item() {
        // module item 焦点为 TweetWithVisibilityResults wrapper，rest_id 在 result.tweet 下。
        let resp = json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                {
                    "type": "TimelineModule",
                    "entries": [{
                        "entryId": "conversationthread-1",
                        "content": { "items": [{
                            "item": { "itemContent": { "tweet_results": { "result": {
                                "__typename": "TweetWithVisibilityResults",
                                "tweet": {
                                    "__typename": "Tweet",
                                    "rest_id": "42",
                                    "legacy": { "full_text": "yo" }
                                }
                            }}}}
                        }]}
                    }]
                }
            ]}}
        });
        let entry = parse_tweet_detail_response(&resp, "42").unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, "42");
    }

    #[test]
    fn parse_soft_fallback_skips_reply_picks_focal() {
        // 一个 module entry，items 含两条：先一条回复（rest_id ≠ focal），后一条焦点。
        // 软兜底应取焦点那条，不是第一条回复。
        let focal_id = "777";
        let resp = json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                {
                    "type": "TimelineModule",
                    "entries": [{
                        "entryId": "conversationthread-777",
                        "content": { "items": [
                            {
                                "item": { "itemContent": { "tweet_results": { "result": {
                                    "__typename": "Tweet",
                                    "rest_id": "111",
                                    "legacy": { "full_text": "reply" }
                                }}}}
                            },
                            {
                                "item": { "itemContent": { "tweet_results": { "result": {
                                    "__typename": "Tweet",
                                    "rest_id": "777",
                                    "legacy": { "full_text": "focal" }
                                }}}}
                            }
                        ]}
                    }]
                }
            ]}}
        });
        let entry = parse_tweet_detail_response(&resp, focal_id).unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        assert_eq!(summary.id, focal_id);
    }

    #[test]
    fn parse_no_match_returns_none() {
        let resp = json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [
                {
                    "type": "TimelineAddEntries",
                    "entries": [{
                        "entryId": "tweet-111",
                        "content": { "itemContent": { "tweet_results": { "result": {
                            "rest_id": "111"
                        }}}}
                    }]
                }
            ]}}
        });
        assert!(parse_tweet_detail_response(&resp, "999").is_none());
    }

    #[test]
    fn focal_typename_detects_tombstone_and_unavailable() {
        let tombstone = json!({
            "content": { "itemContent": { "tweet_results": { "result": {
                "__typename": "TweetTombstone"
            }}}}
        });
        assert_eq!(focal_typename(&tombstone), Some("TweetTombstone"));
        assert!(!focal_has_legacy(&tombstone));

        let unavailable = json!({
            "content": { "itemContent": { "tweet_results": { "result": {
                "__typename": "TweetUnavailable"
            }}}}
        });
        assert_eq!(focal_typename(&unavailable), Some("TweetUnavailable"));
    }

    #[test]
    fn fetch_output_schema_matches_list_likes_tweet() {
        // 离线断言 fetch_tweet 产出的 tweet 字段集与 list_likes 的 tweets[] 元素同构。
        let resp: Value = serde_json::from_str(NEW_2026).unwrap();
        let entry = parse_tweet_detail_response(&resp, "1700000000000039996").unwrap();
        let summary = entry_to_summary(&entry, false).unwrap();
        let out = FetchTweetOutput {
            tweet: summary,
            schema_version: SCHEMA_VERSION,
        };
        let tweet_json = serde_json::to_value(&out.tweet).unwrap();
        let obj = tweet_json.as_object().unwrap();
        for key in [
            "id",
            "author_handle",
            "author_display_name",
            "text",
            "created_at",
            "tweet_url",
            "is_retweet",
            "is_reply",
            "media",
        ] {
            assert!(obj.contains_key(key), "缺字段 {}", key);
        }
        // liked_at 在 TweetDetail 下应缺失（skip_serializing_if）。
        assert!(!obj.contains_key("liked_at"), "TweetDetail 不应含 liked_at");
    }

    // ---- 8.3 classify_tweet_detail ----

    #[test]
    fn classify_401_is_auth_expired() {
        let p = classify_tweet_detail(401, &Value::Null).unwrap();
        assert_eq!(p.kind, ErrorKind::AuthExpired);
    }

    #[test]
    fn classify_403_is_auth_expired() {
        let p = classify_tweet_detail(403, &Value::Null).unwrap();
        assert_eq!(p.kind, ErrorKind::AuthExpired);
    }

    #[test]
    fn classify_404_is_endpoint_stale() {
        let p = classify_tweet_detail(404, &Value::Null).unwrap();
        assert_eq!(p.kind, ErrorKind::EndpointStale);
    }

    #[test]
    fn classify_200_with_auth_error_code_is_auth_expired() {
        let resp = json!({ "errors": [{ "code": 32, "message": "Could not authenticate" }] });
        let p = classify_tweet_detail(200, &resp).unwrap();
        assert_eq!(p.kind, ErrorKind::AuthExpired);
    }

    #[test]
    fn classify_non_json_body_401_is_auth_expired_not_network() {
        // 非 JSON body 被上游解析为 Value::Null；status 401 仍应判 auth_expired。
        let p = classify_tweet_detail(401, &Value::Null).unwrap();
        assert_eq!(p.kind, ErrorKind::AuthExpired);
    }

    #[test]
    fn classify_200_missing_conversation_is_endpoint_stale() {
        let resp = json!({ "data": {} });
        let p = classify_tweet_detail(200, &resp).unwrap();
        assert_eq!(p.kind, ErrorKind::EndpointStale);
    }

    #[test]
    fn classify_200_valid_conversation_passes() {
        let resp = json!({
            "data": { "threaded_conversation_with_injections_v2": { "instructions": [] } }
        });
        assert!(classify_tweet_detail(200, &resp).is_none());
    }

    #[test]
    fn classify_real_fixtures_pass_phase_one() {
        for fx in [NEW_2026, OLD_2023, OLD_2024] {
            let resp: Value = serde_json::from_str(fx).unwrap();
            assert!(classify_tweet_detail(200, &resp).is_none());
        }
    }
}
