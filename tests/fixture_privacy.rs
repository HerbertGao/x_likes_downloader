//! Fixture privacy guard.
//!
//! 目的：防止后续误把"未脱敏"的 TweetDetail 样本提交进 `tests/fixtures/tweet_detail/`。
//! 这三份 fixture 已脱敏（handle / 显示名 / 正文 / 媒体 URL / hashtag / 账号 id 全替换为占位）。
//! 本测试只读断言它们仍不含已知泄露词与非占位账号 id；fixtures 内容不被本测试修改。
//!
//! 不依赖网络、不依赖凭据，纯离线字符串/正则断言。

use std::fs;
use std::path::PathBuf;

use regex::Regex;

/// 自包含的标准 Base64 解码（避免为测试引入新 crate 依赖）。
/// 用于把 `VXNlcj…` node id 解回 `User:<id>` 字节做占位形状断言。
fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    for &b in &bytes {
        let v = lut[b as usize];
        if v == 255 {
            return Err(format!("invalid base64 char {:?}", b as char));
        }
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Ok(out)
}

/// 已知泄露词黑名单：真实媒体/短链域名片段 + 露骨词。
/// 任一出现即说明 fixture 未脱敏。
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "twimg",
    "pic.x.com",
    "t.co/",
    "pic.twitter",
    "吃屎",
    "马桶",
    "黄金",
    "圣水",
    "直男",
];

/// 占位账号 id 前缀：脱敏后的账号 id 为 `900000` + 3 位数字（9 位精确形状）。
///
/// 完整判据见 [`is_placeholder_id`] / [`is_placeholder_payload`]；本常量只用于拼错误信息
/// 与「账号命名空间」的文档说明（时间线占位 id 走 `1700000000000031` + 3 位）。
const PLACEHOLDER_USER_ID_PREFIX: &str = "900000";

fn fixture_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tweet_detail");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixtures/tweet_detail dir readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let all_entries = fs::read_dir(&dir)
        .expect("fixtures/tweet_detail dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(
        (files.len(), all_entries),
        (3, 3),
        "expected exactly 3 tweet_detail json fixtures and no extra entries, got {} files / {} entries: {files:?}",
        files.len(),
        all_entries
    );
    files
}

/// 全部 fixture 目录（tweet_detail + user_tweets），供只读字符串扫描共用。
fn all_fixture_files() -> Vec<PathBuf> {
    let mut v = fixture_files();
    v.extend(user_tweets_fixture_files());
    v
}

#[test]
fn fixtures_contain_no_known_leak_substrings() {
    for path in all_fixture_files() {
        let body = fs::read_to_string(&path).expect("fixture readable");
        for needle in FORBIDDEN_SUBSTRINGS {
            assert!(
                !body.contains(needle),
                "fixture {} contains forbidden substring {:?} — sample is not de-identified",
                path.display(),
                needle
            );
        }
    }
}

/// 断言文件中所有 `user_id_str` / `in_reply_to_user_id_str` 的数字值均为占位。
///
/// 判据是**精确形状**（[`is_placeholder_id`]），不是前缀：`900000` + 3 位（账号）
/// 或 `1700000000000031` + 3 位（时间线）；`9000002123034041339482112` 这类
/// 「真雪花粘占位前缀」必须被拒。
#[test]
fn fixtures_account_ids_are_placeholders() {
    // 匹配 `"user_id_str"` 与 `"in_reply_to_user_id_str"`（冒号后可有空白）后面的数字字符串值。
    let re = Regex::new(r#""(?:in_reply_to_)?user_id_str"\s*:\s*"(\d+)""#).unwrap();
    for path in fixture_files() {
        let body = fs::read_to_string(&path).expect("fixture readable");
        let mut seen = 0usize;
        for cap in re.captures_iter(&body) {
            seen += 1;
            let id = &cap[1];
            // 精确形状而非前缀：`9000002123034041339482112`（真雪花粘占位前缀）必须被拒。
            assert!(
                is_placeholder_id(id),
                "fixture {} has non-placeholder account id {:?} (expected {} + 3 digits, or 1700000000000031 + 3 digits)",
                path.display(),
                id,
                PLACEHOLDER_USER_ID_PREFIX
            );
        }
        assert!(
            seen > 0,
            "fixture {} has no user_id_str at all — structure unexpected",
            path.display()
        );
    }
}

/// 断言 Base64 `User:<id>` node id 解码后均为占位（精确形状，见 [`is_placeholder_payload`]）。
/// 前缀不足：`User:9000002123034041339482112`（真雪花粘占位前缀）也必须被拒。
#[test]
fn fixtures_base64_user_node_ids_are_placeholders() {
    // `VXNlcjo` 是 `User:` 的 Base64 前缀；后跟账号 id 的 Base64。
    let re = Regex::new(r"VXNlcj[A-Za-z0-9+/]+={0,2}").unwrap();
    for path in fixture_files() {
        let body = fs::read_to_string(&path).expect("fixture readable");
        let mut seen = 0usize;
        for m in re.find_iter(&body) {
            seen += 1;
            let token = m.as_str();
            let decoded = b64_decode(token).unwrap_or_else(|e| {
                panic!(
                    "{}: undecodable User node id {token:?}: {e}",
                    path.display()
                )
            });
            let decoded = String::from_utf8(decoded).unwrap_or_else(|e| {
                panic!("{}: non-utf8 User node id {token:?}: {e}", path.display())
            });
            assert!(
                is_placeholder_payload(&decoded),
                "fixture {} has non-placeholder User node id decoding to {:?} \
                 (expected User:{} + 3 digits)",
                path.display(),
                decoded,
                PLACEHOLDER_USER_ID_PREFIX
            );
        }
        // 假绿防线：fixture 若不再含 node id，本测试必须失败而不是静默通过。
        assert!(
            seen > 0,
            "fixture {} has no VXNlcj… User node id — guard would be vacuous",
            path.display()
        );
    }
}

// ==== user_tweets fixtures（2026-11） ====
//
// fixtures 为**手工编写的骨架**（只含解析器读取的键 + 守卫用的 permalink，值全为占位），
// 不由真实抓包转换而来——这消除了「换个编码层就漏」的复发类。
// 守卫为值级默认拒绝：对每个字符串值检查数字串 / 不透明 base64 / URL 主机，
// 并对解析器读取的文本身份字段（*screen_name / name / full_text|text|description）做占位白名单。
// 只扫字符串值：X 的 id 均为字符串，数字或对象键位不在威胁模型内。

fn user_tweets_fixture_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("user_tweets");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixtures/user_tweets dir readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    // 目录条目数也必须是 3：防止新增子目录 / 非 .json 抓包文件绕过值级扫描。
    let all_entries = fs::read_dir(&dir)
        .expect("fixtures/user_tweets dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(
        (files.len(), all_entries),
        (3, 3),
        "expected exactly 3 user_tweets json fixtures and no extra entries, got {} files / {} entries: {files:?}",
        files.len(),
        all_entries
    );
    files
}

/// 收集 (最近的键名, 字符串值)。
fn collect_keyed_strings(
    v: &serde_json::Value,
    key: Option<&str>,
    out: &mut Vec<(Option<String>, String)>,
) {
    match v {
        serde_json::Value::String(s) => out.push((key.map(str::to_string), s.clone())),
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_keyed_strings(x, key, out)),
        serde_json::Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| collect_keyed_strings(x, Some(k), out)),
        _ => {}
    }
}

fn is_placeholder_id(id: &str) -> bool {
    // 精确形状，而非前缀：`9000002123034041339482112`（真雪花粘占位前缀）必须被拒。
    // 两个命名空间：账号 `900000` + 3 位；时间线 `1700000000000031` + 3 位。
    // 后者只提供 1000 个取值，fixture 需要更多推文 id 时应扩宽前缀，而不是放宽 \d{3}。
    Regex::new(r"^(?:900000\d{3}|1700000000000031\d{3})$")
        .unwrap()
        .is_match(id)
}

/// base64 解码载荷（`User:<id>` / `NoteTweet:<id>`）中的 id 必须精确占位。
fn is_placeholder_payload(decoded: &str) -> bool {
    Regex::new(r"^(?:User|NoteTweet):(?:900000\d{3}|1700000000000031\d{3})$")
        .unwrap()
        .is_match(decoded)
}

/// 若字符串内嵌「不透明 base64 载荷」（解码出 `:` 或 ≥10 位连续数字），返回解码文本。
fn opaque_base64_payloads(s: &str) -> Vec<String> {
    let re = Regex::new(r"[A-Za-z0-9+/]{16,}={0,2}").unwrap();
    let digits = Regex::new(r"\d{10,}").unwrap();
    re.find_iter(s)
        .filter_map(|m| b64_decode(m.as_str()).ok())
        .filter_map(|b| String::from_utf8(b).ok())
        .filter(|t| t.contains(':') || digits.is_match(t))
        .collect()
}

#[test]
fn opaque_payload_detector_flags_known_leak_shapes() {
    let note_tweet = b64_encode("NoteTweet:2123034041339482112");
    assert_eq!(
        opaque_base64_payloads(&note_tweet),
        vec!["NoteTweet:2123034041339482112".to_string()],
        "detector must flag NoteTweet:<snowflake>"
    );
    let user_node = b64_encode("User:44196397");
    assert_eq!(opaque_base64_payloads(&user_node).len(), 1);
    assert!(opaque_base64_payloads("fixture_user_a").is_empty());
    assert!(opaque_base64_payloads("TimelineAddEntries").is_empty());
}

fn b64_encode(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[derive(Default, Debug)]
struct Seen {
    long_ids: usize,
    hosts: usize,
    permalinks: usize,
    screen_names: usize,
    names: usize,
    texts: usize,
}

/// 单字符串守卫：Err 表示检测到非占位载荷；Ok 报告命中的可统计类别（防空转）。
fn check_string(key: Option<&str>, s: &str) -> Result<Seen, String> {
    let long_id_re = Regex::new(r"\d{5,}").unwrap();
    let permalink_re = Regex::new(r"/status/(\d+)").unwrap();
    // 主机名：scheme 可选；允许图片/视频文件名形态（纯文件名不是主机）。
    let host_re =
        Regex::new(r#"(?i)(?:https?://)?((?:[a-z0-9-]+\.)+[a-z]{2,})(?:[/:\\"?#\s.,;!)\]}>'|]|$)"#)
            .unwrap();
    let file_ext_re = Regex::new(r"(?i)\.(jpg|jpeg|png|webp|gif|mp4|m3u8)$").unwrap();

    let mut seen = Seen::default();

    // 显式 User node id 形态：与长度无关，始终解码校验。
    if s.contains("VXNlcj") {
        for token in Regex::new(r"VXNlcj[A-Za-z0-9+/]+={0,2}")
            .unwrap()
            .find_iter(s)
        {
            let decoded = b64_decode(token.as_str())
                .map_err(|e| format!("undecodable User node id {token:?}: {e}"))?;
            let decoded = String::from_utf8(decoded)
                .map_err(|e| format!("non-utf8 User node id {token:?}: {e}"))?;
            if !is_placeholder_payload(&decoded) {
                return Err(format!("non-placeholder User node id {decoded:?} in {s:?}"));
            }
        }
    }

    match key {
        Some(k) if k.ends_with("screen_name") => {
            seen.screen_names += 1;
            if !Regex::new(r"^fixture_[a-z0-9_]+$").unwrap().is_match(s) {
                return Err(format!("{k} must be a placeholder, got {s:?}"));
            }
        }
        Some("name") => {
            seen.names += 1;
            // 锚定匹配，拒绝「前置 Fixture 但保留真名」的半脱敏形态。
            if !Regex::new(r"^Fixture User [A-Z]$").unwrap().is_match(s) {
                return Err(format!("display name must be a placeholder, got {s:?}"));
            }
        }
        Some("full_text" | "text" | "description") => {
            seen.texts += 1;
            // 锚定：`Fixture … placeholder … .`，且仅 ASCII——拒绝「真内容 + placeholder」拼接。
            if !Regex::new(r"^Fixture [\x20-\x7E]*placeholder[\x20-\x7E]*\.$")
                .unwrap()
                .is_match(s)
            {
                return Err(format!("tweet text must be a placeholder, got {s:?}"));
            }
        }
        _ => {}
    }

    for m in long_id_re.find_iter(s) {
        seen.long_ids += 1;
        if !is_placeholder_id(m.as_str()) {
            return Err(format!("non-placeholder long id {:?} in {s:?}", m.as_str()));
        }
    }
    for m in permalink_re.captures_iter(s) {
        seen.permalinks += 1;
        if !is_placeholder_id(&m[1]) {
            return Err(format!(
                "permalink with non-placeholder tweet id {:?} in {s:?}",
                &m[1]
            ));
        }
    }
    for cap in host_re.captures_iter(s) {
        let host = cap[1].to_ascii_lowercase();
        if file_ext_re.is_match(&host) {
            continue;
        }
        seen.hosts += 1;
        if !(host == "example.com" || host.ends_with(".example.com") || host == "fixtures.invalid")
        {
            return Err(format!(
                "non-allowlisted host {host:?} (scheme-optional) in {s:?}"
            ));
        }
    }
    for decoded in opaque_base64_payloads(s) {
        if !is_placeholder_payload(&decoded) {
            return Err(format!(
                "opaque base64 payload decoded to non-placeholder {decoded:?} in {s:?}"
            ));
        }
    }
    Ok(seen)
}

/// 守卫自检：round-2/3 实际漏过的泄漏形态必须全部被拒（证明守卫非空转）。
#[test]
fn guard_rejects_known_leak_shapes() {
    let note_tweet = b64_encode("NoteTweet:2123034041339482112");
    let leaks: Vec<(&str, Option<&str>)> = vec![
        (
            "https://twitter.com/realhandle/status/2123034041339482112",
            None,
        ),
        ("twitter.com/realhandle/status/2123034041339482112", None),
        ("xfreeze.com", None),
        ("xfreeze.com/abc123", None),
        ("pbs.twimg.com/media/ABC.jpg", None),
        ("Terafab.AI", None),
        (note_tweet.as_str(), None),
        ("13_2123033978081026048", None),
        // 长度下限会漏掉的短 base64 User: node id（User:123456）。
        // 单独断言走的是 VXNlcj 解码分支，而非别的规则；否则该分支回归会被掩盖。
        ("VXNlcjoxMjM0NTY=", None),
        ("realhandle", Some("screen_name")),
        ("Elon Musk", Some("name")),
        ("马斯克宣布了新的火箭计划", Some("full_text")),
        // 短数字 id（15 位以下）与「半脱敏」形态
        ("44196397", Some("rest_id")),
        ("783214", Some("user_id_str")),
        ("Fixture Elon Musk", Some("name")),
        ("Fixture 马斯克", Some("name")),
        ("马斯克宣布了新的火箭计划 (placeholder)", Some("full_text")),
        ("图片来自 pbs.twimg.com.", None),
        ("1234567890", Some("conversation_id_str")),
        ("987654321", Some("quoted_status_id_str")),
        ("conversationthread-1234567890", Some("entryId")),
        ("realpersonname", Some("in_reply_to_screen_name")),
        ("9000002123034041339482112", Some("rest_id")),
    ];
    for (s, key) in leaks {
        assert!(
            check_string(key, s).is_err(),
            "guard must reject leaked shape {s:?} (key={key:?})"
        );
    }
    let err = check_string(None, "VXNlcjoxMjM0NTY=").unwrap_err();
    assert!(
        err.contains("non-placeholder User node id"),
        "短 base64 必须由 VXNlcj 解码分支拦下，实际: {err}"
    );
    // 合法占位样例必须通过
    for (s, key) in [
        ("fixture_user_a", Some("screen_name")),
        ("Fixture User A", Some("name")),
        (
            "Fixture tweet text (placeholder, de-identified).",
            Some("full_text"),
        ),
        ("https://example.com/fixture_media_1.jpg", None),
        (
            "example.com/fixture_user_c/status/1700000000000031004",
            None,
        ),
        ("FIXTURE_CURSOR_1", None),
    ] {
        assert!(
            check_string(key, s).is_ok(),
            "placeholder value must pass: {s:?} (key={key:?})"
        );
    }
}

/// 值级默认拒绝 + 文本身份白名单。
#[test]
fn user_tweets_fixtures_strings_are_placeholder_only() {
    let mut strings: Vec<(Option<String>, String)> = Vec::new();
    for path in user_tweets_fixture_files() {
        let body = fs::read_to_string(&path).expect("fixture readable");
        let v: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|e| panic!("{}: invalid JSON: {e}", path.display()));
        collect_keyed_strings(&v, None, &mut strings);
    }
    assert!(
        strings.len() > 10,
        "guard is vacuous: only {} strings collected",
        strings.len()
    );

    let mut total = Seen::default();
    for (key, s) in &strings {
        let seen = check_string(key.as_deref(), s).unwrap_or_else(|e| panic!("{e}"));
        total.long_ids += seen.long_ids;
        total.hosts += seen.hosts;
        total.permalinks += seen.permalinks;
        total.screen_names += seen.screen_names;
        total.names += seen.names;
        total.texts += seen.texts;
    }
    assert!(
        total.long_ids > 0
            && total.hosts > 0
            && total.permalinks > 0
            && total.screen_names > 0
            && total.names > 0
            && total.texts > 0,
        "guard is vacuous: {total:?}"
    );
}
