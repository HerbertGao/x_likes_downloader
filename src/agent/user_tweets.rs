use serde_json::Value;

use crate::config::Config;
use crate::error::{classify_status, ErrorKind, ErrorPayload};
use crate::x_api::XApi;

use super::list_likes::{entry_to_summary, parse_likes_response};
use super::types::{UserBrief, UserTweetsOutput, UserTweetsRequest};

const SCHEMA_VERSION: u32 = 1;

/// 鉴权类 GraphQL error code（X 对失效凭据常返回 HTTP 200 + 这些 code）。
/// 与 `fetch_tweet` 保持同一套分类依据。
const AUTH_ERROR_CODES: [i64; 3] = [32, 64, 89];

/// 校验并归一化 username（信任边界，纯函数）。
///
/// 允许可选前导 `@`（CLI/MCP 输入习惯），主体须为 X 规则的
/// `[A-Za-z0-9_]{1,15}`；否则 `invalid_argument`，不做网络往返。
pub fn normalize_username(input: &str) -> Result<String, ErrorPayload> {
    let body = input.strip_prefix('@').unwrap_or(input);
    let valid = !body.is_empty()
        && body.len() <= 15
        && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(body.to_string())
    } else {
        Err(ErrorPayload::new(
            ErrorKind::InvalidArgument,
            format!(
                "username 非法: {:?}（须为 [A-Za-z0-9_]{{1,15}}，可带前导 @）",
                input
            ),
        ))
    }
}

/// 从 UserByScreenName 响应提取用户摘要（纯函数）。
///
/// `data.user` 缺失（不存在/注销/被封，X 返回 HTTP 200 + `{"data":{}}`）或
/// `result.rest_id` 缺失（含 `__typename: UserUnavailable`）→ `None`。
/// `screen_name` 优先取 `core.screen_name`（当前 schema），回退 `legacy.screen_name`（旧 schema）。
pub fn parse_user_by_screen_name(resp: &Value) -> Option<UserBrief> {
    let result = resp.get("data")?.get("user")?.get("result")?;
    let rest_id = result.get("rest_id").and_then(|v| v.as_str())?;
    let core = result.get("core");
    let screen_name = core
        .and_then(|c| c.get("screen_name"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            result
                .get("legacy")
                .and_then(|l| l.get("screen_name"))
                .and_then(|v| v.as_str())
        })?;
    let name = core
        .and_then(|c| c.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(UserBrief {
        screen_name: screen_name.to_string(),
        rest_id: rest_id.to_string(),
        name,
    })
}

/// HTTP 200 + `errors[].code ∈ {32,64,89}` → 凭据失效（与 fetch_tweet 同依据）。
fn has_auth_error(resp: &Value) -> bool {
    resp.get("errors")
        .and_then(|e| e.as_array())
        .map(|errs| {
            errs.iter().any(|err| {
                err.get("code")
                    .and_then(|c| c.as_i64())
                    .map(|code| AUTH_ERROR_CODES.contains(&code))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 按 status 分类非 2xx（同 list_likes：rate_limited 附 60s retry_after）。
fn classify_http_error(status: u16) -> Option<ErrorPayload> {
    let kind = classify_status(status)?;
    let mut payload = ErrorPayload::new(kind, format!("HTTP {}", status));
    if kind == ErrorKind::RateLimited {
        payload = payload.with_retry_after(60);
    }
    Some(payload)
}

/// 按 username 拉取指定账号的推文列表，`tweets[]` 与 `list_likes` 严格同构。
///
/// 链路：UserByScreenName（username → rest_id）→ UserTweets（按 `cursor-bottom-`
/// 游标翻页，语义同 list_likes：`all=false` 单页、`since_cursor` 增量、无新 cursor 即停）。
/// 已知边界：仅扫 `TimelineAddEntries`（置顶 `TimelinePinEntry` 与会话折叠线程均不进入结果）。
/// UserTweets 的 timeline 结构与 Likes 同构，复用 `list_likes::parse_likes_response`
/// 提取顶层 `tweet-*` entry（不含 `profile-conversation-*` 折叠线程）。
pub async fn user_tweets(req: UserTweetsRequest) -> Result<UserTweetsOutput, ErrorPayload> {
    let username = normalize_username(&req.username)?;

    let config = Config::load()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, format!("加载配置失败: {}", e)))?;

    if !config.is_configured() {
        return Err(ErrorPayload::new(
            ErrorKind::NotConfigured,
            "本地未导入凭据",
        ));
    }

    // 先取出 count，后面构造 api 时即可 move config（避免整份 Config clone）。
    let count = req
        .count
        .unwrap_or_else(|| config.count.parse::<u32>().unwrap_or(20));

    let api = XApi::new(config)
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    // 1) username → 用户摘要
    let (status, resp) = api.get_user_by_screen_name(&username).await?;
    if let Some(payload) = classify_http_error(status) {
        return Err(payload);
    }
    if has_auth_error(&resp) {
        return Err(ErrorPayload::new(
            ErrorKind::AuthExpired,
            "UserByScreenName 返回 HTTP 200 但含鉴权类 errors",
        ));
    }
    // graphql_get 把非 JSON body 解析为 Null（如 Cloudflare / 同意页）。这不是「用户不存在」，
    // 不能给出「不要重试」的错误指引。
    if resp.is_null() {
        return Err(ErrorPayload::new(
            ErrorKind::NetworkError,
            "UserByScreenName 返回非 JSON 响应（可能被网络中间层拦截），可重试",
        ));
    }
    let user = parse_user_by_screen_name(&resp).ok_or_else(|| {
        ErrorPayload::new(
            ErrorKind::TweetUnavailable,
            format!("用户不存在、已注销、被封禁或不可见: {}", username),
        )
    })?;

    // 2) rest_id → 推文列表（翻页语义同 list_likes）
    // shortcut: X 的 UserTweets 忽略过小的 count，每页固定返回约 20 条；count 仍按请求透传，真正分页靠 cursor。
    let mut all_summaries = Vec::new();
    let mut all_raw: Vec<Value> = Vec::new();
    let mut cursor = req.since_cursor.clone();

    loop {
        let (status, resp) = api
            .get_user_tweets(&user.rest_id, count as i64, cursor.as_deref())
            .await?;
        if let Some(payload) = classify_http_error(status) {
            return Err(payload);
        }
        if has_auth_error(&resp) {
            return Err(ErrorPayload::new(
                ErrorKind::AuthExpired,
                "UserTweets 返回 HTTP 200 但含鉴权类 errors",
            ));
        }
        if resp.is_null() {
            return Err(ErrorPayload::new(
                ErrorKind::NetworkError,
                "UserTweets 返回非 JSON 响应（可能被网络中间层拦截），可重试",
            ));
        }

        let (entries, new_cursor) = parse_likes_response(&resp);
        for entry in entries {
            if let Some(summary) = entry_to_summary(&entry, req.include_raw) {
                all_summaries.push(summary);
                if req.include_raw {
                    all_raw.push(entry);
                }
            }
        }

        if !req.all {
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

    Ok(UserTweetsOutput {
        user,
        tweets: all_summaries,
        cursor,
        schema_version: SCHEMA_VERSION,
        raw_entries: if req.include_raw { Some(all_raw) } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBSN_OK: &str = include_str!("../../tests/fixtures/user_tweets/ubsn_ok.json");
    const UBSN_NOTFOUND: &str = include_str!("../../tests/fixtures/user_tweets/ubsn_notfound.json");
    const USERTWEETS_OK: &str = include_str!("../../tests/fixtures/user_tweets/usertweets_ok.json");

    // ---- normalize_username ----

    #[test]
    fn normalize_strips_leading_at() {
        assert_eq!(
            normalize_username("@fixture_user_a").unwrap(),
            "fixture_user_a"
        );
        assert_eq!(
            normalize_username("fixture_user_a").unwrap(),
            "fixture_user_a"
        );
    }

    #[test]
    fn normalize_rejects_invalid() {
        for bad in ["", "@", "a b", "a-b", "股", "this_is_way_too_long_for_x"] {
            assert!(normalize_username(bad).is_err(), "{:?} 应被判非法", bad);
        }
    }

    #[test]
    fn normalize_accepts_15_char_underscore() {
        assert_eq!(
            normalize_username("@ab_cd0123456789").unwrap(),
            "ab_cd0123456789"
        );
    }

    // ---- parse_user_by_screen_name ----

    #[test]
    fn parse_ubsn_ok_extracts_user_brief() {
        let resp: Value = serde_json::from_str(UBSN_OK).unwrap();
        let u = parse_user_by_screen_name(&resp).unwrap();
        assert_eq!(u.screen_name, "fixture_user_a");
        assert_eq!(u.rest_id, "900000001");
        assert_eq!(u.name.as_deref(), Some("Fixture User A"));
    }

    #[test]
    fn parse_ubsn_notfound_returns_none() {
        // 用户不存在：HTTP 200 + {"data":{}}（按 2026-11 真实响应形状手工构造）
        let resp: Value = serde_json::from_str(UBSN_NOTFOUND).unwrap();
        assert!(parse_user_by_screen_name(&resp).is_none());
    }

    #[test]
    fn parse_ubsn_unavailable_typename_returns_none() {
        // 被封/受限：data.user.result 存在但 __typename=UserUnavailable、无 rest_id
        let resp: Value = serde_json::from_str(
            r#"{"data":{"user":{"result":{"__typename":"UserUnavailable","reason":"Suspended"}}}}"#,
        )
        .unwrap();
        assert!(parse_user_by_screen_name(&resp).is_none());
    }

    #[test]
    fn parse_ubsn_legacy_screen_name_fallback() {
        // 旧 schema：screen_name 在 legacy 下而非 core
        let resp: Value = serde_json::from_str(
            r#"{"data":{"user":{"result":{"rest_id":"123","legacy":{"screen_name":"old_schema"}}}}}"#,
        )
        .unwrap();
        let u = parse_user_by_screen_name(&resp).unwrap();
        assert_eq!(u.screen_name, "old_schema");
    }

    // ---- UserTweets 响应（手工骨架，仅保留解析器读取的键；2026-11 schema） ----

    #[test]
    fn parse_user_tweets_entries_and_cursor() {
        let resp: Value = serde_json::from_str(USERTWEETS_OK).unwrap();
        let (entries, cursor) = parse_likes_response(&resp);
        assert_eq!(entries.len(), 2);
        assert!(entries[0]
            .get("entryId")
            .and_then(|v| v.as_str())
            .unwrap()
            .starts_with("tweet-"));
        assert_eq!(cursor.as_deref(), Some("FIXTURE_CURSOR_1"));
        // entry 能转成与 list_likes 同构的 TweetSummary
        let summary = entry_to_summary(&entries[0], false).unwrap();
        assert_eq!(summary.id, "1700000000000031002");
        assert_eq!(summary.author_handle, "fixture_user_a");
        assert_eq!(summary.media.len(), 1, "普通推文应解析出 1 个媒体");

        // RT 边界：外层 legacy 无 extended_entities → media 为空；内层被转推文确有媒体。
        // 这条断言钉住 SKILL.md 声明的「RT 条目 media[] 恒为空」行为。
        let rt = entry_to_summary(&entries[1], false).unwrap();
        assert!(rt.is_retweet);
        assert!(
            rt.media.is_empty(),
            "RT 条目 media[] 必须为空（内层媒体不展开）"
        );
        // 置顶推文在 TimelinePinEntry 指令里，必须被排除（fixture 特意带了该指令）。
        let instructions = resp["data"]["user"]["result"]["timeline"]["timeline"]["instructions"]
            .as_array()
            .unwrap();
        assert!(
            instructions.iter().any(|i| i["type"] == "TimelinePinEntry"),
            "fixture 必须含置顶指令，否则该断言是空转的"
        );
        let add_entries = instructions
            .iter()
            .find(|i| i["type"] == "TimelineAddEntries")
            .expect("fixture 必须含 TimelineAddEntries");
        let inner_media = add_entries["entries"][1]["content"]["itemContent"]["tweet_results"]
            ["result"]["legacy"]["retweeted_status_result"]["result"]["legacy"]
            ["extended_entities"]["media"]
            .as_array()
            .expect("fixture 的内层 RT 必须带媒体");
        assert!(!inner_media.is_empty(), "fixture 的内层 RT 媒体不得为空");
    }

    #[test]
    fn has_auth_error_matches_x_codes() {
        let resp: Value =
            serde_json::from_str(r#"{"errors":[{"code":32,"message":"Not authorized"}]}"#).unwrap();
        assert!(has_auth_error(&resp));
        let resp: Value = serde_json::from_str(r#"{"errors":[{"code":999}]}"#).unwrap();
        assert!(!has_auth_error(&resp));
        let resp: Value = serde_json::from_str("{}").unwrap();
        assert!(!has_auth_error(&resp));
    }
}
