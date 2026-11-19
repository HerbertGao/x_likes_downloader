use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::config::Config;
use crate::error::{classify_status, ErrorKind, ErrorPayload};
use crate::x_api::XApi;

use super::list_likes::{entry_to_summary, find_outer_timeline, parse_likes_response};
use super::types::{TweetSummary, UserBrief, UserTweetsOutput, UserTweetsRequest};

/// 一页 GraphQL 取数结果 `(http_status, body)`（便于单测注入，不绑死网络）。
type PageResult = Result<(u16, Value), ErrorPayload>;
type PageFuture = Pin<Box<dyn Future<Output = PageResult> + Send>>;

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

/// 非鉴权类顶层 GraphQL 操作错误：仅当**该操作所需的数据**不可用时视为失败。
///
/// `data_usable` 由调用方按操作给出（lookup 看 `user.result`，timeline 看 timeline 节点），
/// 不能只看 `data` 是否为 null：`{"data":{},"errors":[...]}` 与
/// `{"data":{"user":null},"errors":[...]}` 都是「有 errors 但没有可用数据」。
/// X 有时会同时返回可用 `data` 与非致命 `errors`（告警）；此时保留可用内容（容忍降级）。
/// 缺失用户的合法空响应是 `{"data":{}}`（**无 errors**），不命中本函数，
/// 仍由 `parse_user_by_screen_name` → `tweet_unavailable` 处理。
fn operation_error(resp: &Value, data_usable: bool) -> Option<ErrorPayload> {
    let errors = resp.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    if data_usable {
        return None;
    }
    let message = errors
        .first()
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    Some(ErrorPayload::new(
        ErrorKind::NetworkError,
        format!("GraphQL 操作失败（{message}），可重试"),
    ))
}

/// 统一的响应判定：HTTP 状态 → 鉴权类 errors → 非 JSON body → 非鉴权操作错误。
///
/// 顺序很重要：鉴权错误必须先于「操作错误」判定（两者都走 HTTP 200 + errors）。
fn ensure_response_ok(
    status: u16,
    resp: &Value,
    operation: &str,
    data_usable: bool,
) -> Result<(), ErrorPayload> {
    if let Some(payload) = classify_http_error(status) {
        return Err(payload);
    }
    if has_auth_error(resp) {
        return Err(ErrorPayload::new(
            ErrorKind::AuthExpired,
            format!("{operation} 返回 HTTP 200 但含鉴权类 errors"),
        ));
    }
    // graphql_get 把非 JSON body 解析为 Null（如 Cloudflare / 同意页）。这不是「用户不存在」，
    // 不能给出「不要重试」的错误指引。
    if resp.is_null() {
        return Err(ErrorPayload::new(
            ErrorKind::NetworkError,
            format!("{operation} 返回非 JSON 响应（可能被网络中间层拦截），可重试"),
        ));
    }
    if let Some(payload) = operation_error(resp, data_usable) {
        return Err(payload);
    }
    Ok(())
}

/// 解析 UserByScreenName 响应：先统一判定，再把「无 errors 的缺用户」映射为 `tweet_unavailable`。
fn parse_lookup(status: u16, resp: &Value, username: &str) -> Result<UserBrief, ErrorPayload> {
    let user = parse_user_by_screen_name(resp);
    ensure_response_ok(status, resp, "UserByScreenName", user.is_some())?;
    user.ok_or_else(|| {
        ErrorPayload::new(
            ErrorKind::TweetUnavailable,
            format!("用户不存在、已注销、被封禁或不可见: {}", username),
        )
    })
}

/// 解析一页 UserTweets 响应：任何页面级错误都必须 `Err`，不得让调用方把前几页当成完整结果。
fn parse_timeline_page(
    status: u16,
    resp: &Value,
) -> Result<(Vec<Value>, Option<String>), ErrorPayload> {
    // 该页「可用」的判据是 timeline 节点存在，而不是 data 非 null：
    // 最后一页合法地可能没有任何 tweet entry，但 timeline 节点仍在。
    let data_usable = find_outer_timeline(resp).is_some();
    ensure_response_ok(status, resp, "UserTweets", data_usable)?;
    Ok(parse_likes_response(resp))
}

/// 按游标翻页聚合。取数函数注入，便于离线覆盖「第 2 页失败不得返回部分结果」。
///
/// 语义同 `list_likes`：`all=false` 单页；`all=true` 翻到无游标或游标重复为止；
/// `since_cursor` 为起点。任一页 `Err` 立即向上传播（丢弃已聚合页，不返回部分成功）。
async fn collect_pages<F>(
    mut fetch: F,
    all: bool,
    since_cursor: Option<String>,
    include_raw: bool,
) -> Result<(Vec<TweetSummary>, Vec<Value>, Option<String>), ErrorPayload>
where
    F: FnMut(Option<String>) -> PageFuture,
{
    let mut summaries = Vec::new();
    let mut raw_entries: Vec<Value> = Vec::new();
    let mut cursor = since_cursor;

    loop {
        let (status, resp) = fetch(cursor.clone()).await?;
        let (entries, new_cursor) = parse_timeline_page(status, &resp)?;

        for entry in entries {
            if let Some(summary) = entry_to_summary(&entry, include_raw) {
                summaries.push(summary);
                if include_raw {
                    raw_entries.push(entry);
                }
            }
        }

        if !all {
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
            _ => cursor = new_cursor,
        }
    }

    Ok((summaries, raw_entries, cursor))
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

    let api = Arc::new(
        XApi::new(config)
            .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?,
    );

    // 1) username → 用户摘要（含非鉴权操作错误判定，避免误报「用户不存在」）
    let (status, resp) = api.get_user_by_screen_name(&username).await?;
    let user = parse_lookup(status, &resp, &username)?;

    // 2) rest_id → 推文列表（翻页语义同 list_likes）
    // shortcut: X 的 UserTweets 忽略过小的 count，每页固定返回约 20 条；count 仍按请求透传，真正分页靠 cursor。
    let user_id = user.rest_id.clone();
    let (tweets, raw_entries, cursor) = collect_pages(
        move |cursor: Option<String>| {
            let api = Arc::clone(&api);
            let user_id = user_id.clone();
            Box::pin(async move {
                api.get_user_tweets(&user_id, count as i64, cursor.as_deref())
                    .await
            })
        },
        req.all,
        req.since_cursor.clone(),
        req.include_raw,
    )
    .await?;

    Ok(UserTweetsOutput {
        user,
        tweets,
        cursor,
        schema_version: SCHEMA_VERSION,
        raw_entries: if req.include_raw {
            Some(raw_entries)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;

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

    // ---- GraphQL 操作错误分类（review：非鉴权 errors 不得误报用户不存在 / 部分成功） ----

    #[test]
    fn operation_error_is_retryable_when_data_missing() {
        let resp = json!({"errors": [{"message": "Internal server error"}], "data": null});
        let err = operation_error(&resp, false).expect("must be classified as operation error");
        assert_eq!(err.kind, ErrorKind::NetworkError);
    }

    #[test]
    fn operation_error_tolerated_when_data_present() {
        // 告警型 errors + 可用 data：保留可用内容（容忍降级）
        let resp = json!({"data": {"user": {"result": {"rest_id": "900000001"}}}, "errors": [{"message": "partial"}]});
        assert!(operation_error(&resp, true).is_none());
    }

    #[test]
    fn operation_error_ignores_error_free_and_empty_errors() {
        assert!(operation_error(&json!({"data": {}}), false).is_none());
        assert!(operation_error(&json!({"errors": [], "data": null}), false).is_none());
        assert!(operation_error(&json!({"data": null}), false).is_none());
    }

    #[test]
    fn operation_error_flags_empty_data_with_errors() {
        // `data` 非 null 但没有任何可用内容——不能只看 data 是否为 null
        for resp in [
            json!({"data": {}, "errors": [{"message": "Internal server error"}]}),
            json!({"data": {"user": null}, "errors": [{"message": "Internal server error"}]}),
        ] {
            let err = operation_error(&resp, false)
                .unwrap_or_else(|| panic!("must flag empty data with errors: {resp}"));
            assert_eq!(err.kind, ErrorKind::NetworkError);
        }
    }

    #[test]
    fn lookup_empty_data_with_errors_is_retryable_not_unavailable() {
        for resp in [
            json!({"data": {}, "errors": [{"message": "Internal server error"}]}),
            json!({"data": {"user": null}, "errors": [{"message": "Internal server error"}]}),
        ] {
            let err = parse_lookup(200u16, &resp, "fixture_user_a").unwrap_err();
            assert_eq!(
                err.kind,
                ErrorKind::NetworkError,
                "有 errors 但无 user 数据时必须可重试，不能是 tweet_unavailable: {resp}"
            );
        }
    }

    #[test]
    fn timeline_empty_data_with_errors_is_err_not_empty_page() {
        let resp = json!({"data": {}, "errors": [{"message": "Internal server error"}]});
        let err = parse_timeline_page(200u16, &resp).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NetworkError);
    }

    #[test]
    fn timeline_usable_data_with_warnings_is_tolerated() {
        // 有 timeline 节点 + 非致命 errors：保留可用内容
        let resp = page(vec![tweet_entry("1700000000000031002")], None);
        let mut resp = resp;
        resp["errors"] = json!([{"message": "partial"}]);
        let (entries, cursor) = parse_timeline_page(200u16, &resp).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(cursor.is_none());
    }

    #[test]
    fn lookup_operation_error_is_not_reported_as_missing_user() {
        let resp = json!({"errors": [{"message": "Internal server error"}], "data": null});
        let err = parse_lookup(200, &resp, "fixture_user_a").unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::NetworkError,
            "上游操作错误必须是可重试的 network_error，不能是 tweet_unavailable"
        );
    }

    #[test]
    fn lookup_error_free_missing_user_is_unavailable() {
        // 合法缺用户形态：HTTP 200 + {"data":{}}（无 errors）
        let resp = json!({"data": {}});
        let err = parse_lookup(200, &resp, "fixture_user_a").unwrap_err();
        assert_eq!(err.kind, ErrorKind::TweetUnavailable);
    }

    #[test]
    fn lookup_auth_error_wins_over_operation_error() {
        let resp = json!({"errors": [{"code": 32, "message": "Not authorized"}], "data": null});
        let err = parse_lookup(200, &resp, "fixture_user_a").unwrap_err();
        assert_eq!(err.kind, ErrorKind::AuthExpired);
    }

    #[test]
    fn lookup_non_json_is_retryable() {
        let err = parse_lookup(200, &Value::Null, "fixture_user_a").unwrap_err();
        assert_eq!(err.kind, ErrorKind::NetworkError);
    }

    // ---- 翻页（review：第 2 页失败不得返回第 1 页的部分成功） ----

    fn page(entries: Vec<Value>, cursor: Option<&str>) -> Value {
        let mut all = entries;
        if let Some(c) = cursor {
            all.push(json!({
                "entryId": format!("cursor-bottom-{}", c),
                "content": { "value": c }
            }));
        }
        json!({
            "data": { "user": { "result": { "timeline": { "timeline": {
                "instructions": [{ "type": "TimelineAddEntries", "entries": all }]
            } } } } }
        })
    }

    fn tweet_entry(id: &str) -> Value {
        json!({
            "entryId": format!("tweet-{}", id),
            "content": { "itemContent": { "tweet_results": { "result": {
                "rest_id": id,
                "core": { "user_results": { "result": {
                    "rest_id": "900000001",
                    "core": { "screen_name": "fixture_user_a", "name": "Fixture User A" }
                } } },
                "legacy": { "full_text": "Fixture tweet text (placeholder)." }
            } } } }
        })
    }

    #[tokio::test]
    async fn collect_pages_single_page_when_all_false() {
        let mut pages = VecDeque::from(vec![Ok((
            200u16,
            page(vec![tweet_entry("1700000000000031002")], Some("C1")),
        ))]);
        let (tweets, raw, cursor) = collect_pages(
            move |_| {
                let p = pages.pop_front().expect("one page");
                Box::pin(async move { p }) as PageFuture
            },
            false,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(tweets.len(), 1);
        assert_eq!(cursor.as_deref(), Some("C1"));
        assert!(raw.is_empty());
    }

    #[tokio::test]
    async fn collect_pages_failure_after_one_page_returns_err_not_partial() {
        // 第 1 页成功、第 2 页返回非鉴权操作错误：整体必须 Err，不能把第 1 页当完整结果。
        let mut pages = VecDeque::from(vec![
            Ok((
                200u16,
                page(vec![tweet_entry("1700000000000031002")], Some("C1")),
            )),
            Ok((
                200u16,
                json!({"errors": [{"message": "Internal server error"}], "data": null}),
            )),
        ]);
        let result = collect_pages(
            move |_| {
                let p = pages.pop_front().expect("two pages");
                Box::pin(async move { p }) as PageFuture
            },
            true,
            None,
            false,
        )
        .await;
        match result {
            Err(err) => assert_eq!(err.kind, ErrorKind::NetworkError),
            Ok((tweets, _, _)) => panic!(
                "page-2 failure must not return a partial success, got {} tweets",
                tweets.len()
            ),
        }
    }

    #[tokio::test]
    async fn collect_pages_empty_data_with_errors_after_one_page_is_err() {
        // 与 `data: null` 同类的边界：`{"data":{},"errors":[...]}` 也不能被当成「空页正常结束」。
        let mut pages = VecDeque::from(vec![
            Ok((
                200u16,
                page(vec![tweet_entry("1700000000000031002")], Some("C1")),
            )),
            Ok((
                200u16,
                json!({"data": {}, "errors": [{"message": "Internal server error"}]}),
            )),
        ]);
        let result = collect_pages(
            move |_| {
                let p = pages.pop_front().expect("two pages");
                Box::pin(async move { p }) as PageFuture
            },
            true,
            None,
            false,
        )
        .await;
        assert!(
            matches!(result, Err(ref e) if e.kind == ErrorKind::NetworkError),
            "page-2 empty-data-with-errors must fail the whole request, got {result:?}"
        );
    }

    #[tokio::test]
    async fn collect_pages_stops_on_repeated_cursor() {
        let mut pages = VecDeque::from(vec![
            Ok((
                200u16,
                page(vec![tweet_entry("1700000000000031002")], Some("C1")),
            )),
            Ok((
                200u16,
                page(vec![tweet_entry("1700000000000031010")], Some("C1")),
            )),
        ]);
        let (tweets, _, cursor) = collect_pages(
            move |_| {
                let p = pages.pop_front().expect("two pages");
                Box::pin(async move { p }) as PageFuture
            },
            true,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(tweets.len(), 2, "两页各一条，游标重复即停");
        assert_eq!(cursor.as_deref(), Some("C1"));
    }

    #[tokio::test]
    async fn collect_pages_aggregates_until_cursor_none() {
        let mut pages = VecDeque::from(vec![
            Ok((
                200u16,
                page(vec![tweet_entry("1700000000000031002")], Some("C1")),
            )),
            Ok((200u16, page(vec![tweet_entry("1700000000000031010")], None))),
        ]);
        let (tweets, _, cursor) = collect_pages(
            move |_| {
                let p = pages.pop_front().expect("two pages");
                Box::pin(async move { p }) as PageFuture
            },
            true,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(tweets.len(), 2);
        assert_eq!(cursor, None);
    }
}
