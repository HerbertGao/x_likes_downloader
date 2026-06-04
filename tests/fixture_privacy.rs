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
/// 仅用于把 `VXNlcj…` node id 解回 `User:<id>` 字节做前缀断言。
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

/// 占位账号 id 前缀：脱敏后所有数字账号 id 形如 `900000xx`（9 位，`900000` 打头）。
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
    assert_eq!(
        files.len(),
        3,
        "expected exactly 3 tweet_detail fixtures, got {}: {files:?}",
        files.len()
    );
    files
}

#[test]
fn fixtures_contain_no_known_leak_substrings() {
    for path in fixture_files() {
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

/// 断言文件中所有 `user_id_str` / `in_reply_to_user_id_str` 的数字值均为占位（`900000` 打头）。
/// 即不允许出现 `"user_id_str": "<non-900000 digits>"` 形态的真实账号 id。
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
            assert!(
                id.starts_with(PLACEHOLDER_USER_ID_PREFIX),
                "fixture {} has non-placeholder account id {:?} (expected {}…)",
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

/// 断言 Base64 `User:<id>` node id 解码后均为占位（`User:900000…`）。
/// 不允许出现解码为非 `User:900000` 的 `VXNlcj…` Base64（真实账号 node id）。
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
            let decoded = b64_decode(token)
                .unwrap_or_else(|e| panic!("{}: undecodable User node id {token:?}: {e}", path.display()));
            let decoded = String::from_utf8(decoded)
                .unwrap_or_else(|e| panic!("{}: non-utf8 User node id {token:?}: {e}", path.display()));
            assert!(
                decoded.starts_with(&format!("User:{PLACEHOLDER_USER_ID_PREFIX}")),
                "fixture {} has non-placeholder User node id decoding to {:?} (expected User:{}…)",
                path.display(),
                decoded,
                PLACEHOLDER_USER_ID_PREFIX
            );
        }
        if seen == 0 {
            eprintln!(
                "note: {} has no VXNlcj… User node id (acceptable)",
                path.display()
            );
        }
    }
}
