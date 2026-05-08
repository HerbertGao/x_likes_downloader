use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::Path;

use crate::agent::types::ImportOutput;
use crate::config::credentials_path;
use crate::error::{ErrorKind, ErrorPayload};

#[derive(Parser, Clone)]
#[command(name = "setup")]
#[command(about = "初始化X下载器配置")]
pub struct SetupArgs {
    /// curl命令文件路径（位置参数）
    #[arg(default_value = "curl_command.txt")]
    pub curl_file: String,

    /// 等价于位置参数（与文档一致的长选项形式）
    #[arg(long = "curl-file")]
    pub curl_file_long: Option<String>,

    /// 自定义沙箱下载目录（写入本地配置）；不传则保持现有值或使用平台默认
    #[arg(long)]
    pub download_dir: Option<String>,

    /// JSON 信封输出（stdout 单 JSON 对象 + 调试信息走 stderr）
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedCurl {
    pub bearer_token: String,
    pub cookie_str: String,
    pub user_agent: Option<String>,
    pub x_client_uuid: Option<String>,
    pub x_client_transaction_id: Option<String>,
    /// 完整 Likes 端点 URL（含 queryId 路径段，去除 query string）
    pub likes_api_url: String,
    /// URL 中 features 参数解码后的 JSON 字符串
    pub likes_features: String,
    /// URL 中 fieldToggles 参数解码后的 JSON 字符串
    pub likes_fieldtoggles: String,
}

#[derive(Debug)]
pub struct ParsedCookies {
    pub twid: String,
    pub auth_token: String,
    pub ct0: String,
    pub personalization_id: String,
}

pub fn run_setup(args: SetupArgs) -> Result<()> {
    // 读取curl命令文件（--curl-file 优先于位置参数）
    let curl_file = args
        .curl_file_long
        .as_deref()
        .unwrap_or(&args.curl_file)
        .to_string();
    let curl_exists = Path::new(&curl_file).exists();

    // 纯目录更新：用户只想改沙箱 base，没有提供（或不存在）cURL 文件
    if !curl_exists && args.download_dir.is_some() {
        let new_dir = args.download_dir.as_deref().unwrap();
        let cred_path = credentials_path();
        update_download_sandbox_only(new_dir, &cred_path)?;
        println!("沙箱下载目录已更新: {}", new_dir);
        return Ok(());
    }

    if !curl_exists {
        return Err(anyhow::anyhow!(
            "找不到 cURL 文件: {} （要更新沙箱目录请用 --download-dir <path>）",
            curl_file
        ));
    }

    let curl_command =
        fs::read_to_string(&curl_file).with_context(|| format!("读取 {} 失败", curl_file))?;

    // 解析curl命令
    let parsed = parse_curl_command(&curl_command)?;
    let cookies = parse_cookies(&parsed.cookie_str)?;

    // 保存私有令牌（含协议参数）
    let cred_path = credentials_path();
    save_private_tokens(
        &cookies.twid,
        &parsed.bearer_token,
        &cookies.auth_token,
        &cookies.ct0,
        &cookies.personalization_id,
        parsed.user_agent.as_deref().unwrap_or(""),
        parsed.x_client_uuid.as_deref().unwrap_or(""),
        parsed.x_client_transaction_id.as_deref().unwrap_or(""),
        &parsed.likes_api_url,
        &parsed.likes_features,
        &parsed.likes_fieldtoggles,
        args.download_dir.as_deref(),
        &cred_path,
    )?;

    println!("生成 {} 成功！", cred_path.display());
    println!("初始化完成。");
    Ok(())
}

/// JSON 模式入口：解析 cURL 并写入私有令牌，返回结构化结果。
/// 不向 stdout 写任何内容（人类调试信息走 stderr）。
pub fn run_setup_json(args: SetupArgs) -> Result<ImportOutput, ErrorPayload> {
    let curl_file = args.curl_file_long.as_deref().unwrap_or(&args.curl_file);
    let curl_exists = Path::new(curl_file).exists();

    // 纯目录更新：用户只想改沙箱 base，没有提供（或不存在）cURL 文件
    if !curl_exists && args.download_dir.is_some() {
        let new_dir = args.download_dir.as_deref().unwrap();
        let cred_path = credentials_path();
        update_download_sandbox_only(new_dir, &cred_path)
            .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;
        return Ok(ImportOutput {
            written: false,
            path: cred_path,
            protocol_params_extracted: false,
        });
    }

    if !curl_exists {
        return Err(ErrorPayload::new(
            ErrorKind::InvalidArgument,
            format!(
                "找不到 cURL 文件: {} （要更新沙箱目录请用 --download-dir <path>）",
                curl_file
            ),
        ));
    }

    let curl_command = fs::read_to_string(curl_file).map_err(|e| {
        ErrorPayload::new(
            ErrorKind::InvalidArgument,
            format!("读取 {} 失败: {}", curl_file, e),
        )
    })?;
    run_setup_from_text(&curl_command, args.download_dir.as_deref())
}

/// 直接从 cURL 文本（内存中字符串）做 JSON 模式 setup，不写临时文件。
/// 用于 `agent::import_curl` 与未来 MCP server 的 `setup_from_curl` 工具。
///
/// 安全考量：cURL 文本含 cookie / bearer / auth 等敏感凭据，
/// 整个流程仅在内存中处理，不落盘到任何用户可见路径之外的位置。
pub fn run_setup_from_text(
    curl_text: &str,
    download_dir: Option<&str>,
) -> Result<ImportOutput, ErrorPayload> {
    let parsed = parse_curl_command(curl_text)
        .map_err(|e| ErrorPayload::new(ErrorKind::InvalidArgument, e.to_string()))?;
    let cookies = parse_cookies(&parsed.cookie_str)
        .map_err(|e| ErrorPayload::new(ErrorKind::InvalidArgument, e.to_string()))?;

    let cred_path = credentials_path();
    save_private_tokens(
        &cookies.twid,
        &parsed.bearer_token,
        &cookies.auth_token,
        &cookies.ct0,
        &cookies.personalization_id,
        parsed.user_agent.as_deref().unwrap_or(""),
        parsed.x_client_uuid.as_deref().unwrap_or(""),
        parsed.x_client_transaction_id.as_deref().unwrap_or(""),
        &parsed.likes_api_url,
        &parsed.likes_features,
        &parsed.likes_fieldtoggles,
        download_dir,
        &cred_path,
    )
    .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    Ok(ImportOutput {
        written: true,
        path: cred_path,
        protocol_params_extracted: true,
    })
}

pub fn parse_curl_command(curl_command: &str) -> Result<ParsedCurl> {
    let header_regex = Regex::new(r#"-H\s+'([^']+)'"#)?;
    let cookie_regex = Regex::new(r#"-b\s+'([^']+)'"#)?;
    let bearer_regex = Regex::new(r#"Bearer\s+(\S+)"#)?;
    let url_regex = Regex::new(r#"'(https?://[^']+)'|"(https?://[^"]+)""#)?;

    let mut bearer_token = None;
    let mut user_agent = None;
    let mut x_client_uuid = None;
    let mut x_client_transaction_id = None;

    // 解析headers
    for cap in header_regex.captures_iter(curl_command) {
        let header = &cap[1];
        let header_lower = header.to_lowercase();

        if header_lower.starts_with("authorization:") {
            if let Some(bearer_cap) = bearer_regex.captures(header) {
                bearer_token = Some(bearer_cap[1].to_string());
            }
        } else if header_lower.starts_with("user-agent:") {
            user_agent = Some(header.split_once(':').unwrap().1.trim().to_string());
        } else if header_lower.starts_with("x-client-uuid:") {
            x_client_uuid = Some(header.split_once(':').unwrap().1.trim().to_string());
        } else if header_lower.starts_with("x-client-transaction-id:") {
            x_client_transaction_id = Some(header.split_once(':').unwrap().1.trim().to_string());
        }
    }

    // 解析cookie
    let cookie_str = if let Some(cap) = cookie_regex.captures(curl_command) {
        cap[1].to_string()
    } else {
        return Err(anyhow::anyhow!("无法找到cookie参数"));
    };

    let bearer_token = bearer_token.ok_or_else(|| anyhow::anyhow!("无法解析Bearer Token"))?;

    // 解析 URL：找到第一个包含 /Likes 路径段的 URL
    let likes_url_full = url_regex
        .captures_iter(curl_command)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().to_string())
        })
        .find(|u| u.contains("/Likes"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cURL 中未找到 X Likes 端点请求。请确保从浏览器复制的是 Likes API 请求（URL 路径段含 `/Likes`）"
            )
        })?;

    let (likes_api_url, likes_features, likes_fieldtoggles) =
        split_url_and_extract_protocol(&likes_url_full)?;

    Ok(ParsedCurl {
        bearer_token,
        cookie_str,
        user_agent,
        x_client_uuid,
        x_client_transaction_id,
        likes_api_url,
        likes_features,
        likes_fieldtoggles,
    })
}

/// 把完整的 Likes URL 拆成 base URL（去 query string）+ 解码后的 features / fieldToggles JSON 字符串。
fn split_url_and_extract_protocol(full_url: &str) -> Result<(String, String, String)> {
    let parsed = url::Url::parse(full_url).context("解析 Likes URL 失败")?;

    // 校验路径段含 /Likes（防止误匹配如 /LikesUser 这种端点）
    let path_segments: Vec<&str> = parsed
        .path_segments()
        .map(|it| it.collect())
        .unwrap_or_default();
    if !path_segments.contains(&"Likes") {
        return Err(anyhow::anyhow!(
            "URL 路径段不含 `Likes`，请确认导出的是 Likes 端点 cURL"
        ));
    }

    let mut base = parsed.clone();
    base.set_query(None);
    base.set_fragment(None);
    let base_str = base.as_str().trim_end_matches('?').to_string();

    // 提取 features / fieldToggles
    let mut features = None;
    let mut fieldtoggles = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "features" => features = Some(v.into_owned()),
            "fieldToggles" => fieldtoggles = Some(v.into_owned()),
            _ => {}
        }
    }

    let features = features.ok_or_else(|| anyhow::anyhow!("Likes URL 缺少 features query 参数"))?;
    // fieldToggles 在某些情况下可缺失，允许默认值
    let fieldtoggles =
        fieldtoggles.unwrap_or_else(|| r#"{"withArticlePlainText":false}"#.to_string());

    Ok((base_str, features, fieldtoggles))
}

pub fn parse_cookies(cookie_str: &str) -> Result<ParsedCookies> {
    let mut cookies = std::collections::HashMap::new();

    for part in cookie_str.split(';') {
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            cookies.insert(key, value);
        }
    }

    let required_keys = ["twid", "auth_token", "ct0", "personalization_id"];
    let mut result = ParsedCookies {
        twid: String::new(),
        auth_token: String::new(),
        ct0: String::new(),
        personalization_id: String::new(),
    };

    for key in &required_keys {
        let value = cookies
            .get(*key)
            .ok_or_else(|| anyhow::anyhow!("Cookie中缺少必需的字段: {}", key))?;

        match *key {
            "twid" => {
                result.twid = if let Some(stripped) = value.strip_prefix("u=") {
                    stripped.to_string()
                } else {
                    value.to_string()
                };
                if result.twid.starts_with("u%3D") {
                    result.twid = result.twid[4..].to_string();
                }
            }
            "auth_token" => result.auth_token = value.to_string(),
            "ct0" => result.ct0 = value.to_string(),
            "personalization_id" => result.personalization_id = value.to_string(),
            _ => {}
        }
    }

    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn save_private_tokens(
    user_id: &str,
    bearer_token: &str,
    auth_token: &str,
    ct0: &str,
    personalization_id: &str,
    user_agent: &str,
    x_client_uuid: &str,
    x_client_transaction_id: &str,
    likes_api_url: &str,
    likes_features: &str,
    likes_fieldtoggles: &str,
    download_sandbox_base_dir: Option<&str>,
    path: &Path,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // 如果用户没传 --download-dir，保留现有文件中的值（如果有）
    let preserved_sandbox = if download_sandbox_base_dir.is_none() {
        read_existing_field(path, "DOWNLOAD_SANDBOX_BASE_DIR")
    } else {
        None
    };
    let sandbox_value = download_sandbox_base_dir
        .map(|s| s.to_string())
        .or(preserved_sandbox)
        .unwrap_or_default();

    let mut content = format!(
        "USER_ID={}\nBEARER_TOKEN={}\nAUTH_TOKEN={}\nCT0={}\nPERSONALIZATION_ID={}\nUSER_AGENT={}\nX_CLIENT_UUID={}\nX_CLIENT_TRANSACTION_ID={}\nLIKES_API_URL={}\nLIKES_FEATURES={}\nLIKES_FIELDTOGGLES={}\n",
        user_id,
        bearer_token,
        auth_token,
        ct0,
        personalization_id,
        user_agent,
        x_client_uuid,
        x_client_transaction_id,
        likes_api_url,
        likes_features,
        likes_fieldtoggles,
    );
    if !sandbox_value.is_empty() {
        content.push_str(&format!("DOWNLOAD_SANDBOX_BASE_DIR={}\n", sandbox_value));
    }

    fs::write(path, content)?;
    Ok(())
}

/// 仅更新 `DOWNLOAD_SANDBOX_BASE_DIR` 字段，保留其它字段不变。
/// 当文件不存在时创建仅含此字段的新文件。不做任何 stdout/stderr 输出。
fn update_download_sandbox_only(new_dir: &str, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut other_lines: Vec<String> = if path.exists() {
        fs::read_to_string(path)?
            .lines()
            .filter(|l| !l.starts_with("DOWNLOAD_SANDBOX_BASE_DIR="))
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };
    other_lines.push(format!("DOWNLOAD_SANDBOX_BASE_DIR={}", new_dir));
    let mut content = other_lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }
    fs::write(path, content)?;
    Ok(())
}

fn read_existing_field(path: &Path, key: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_extracts_base_and_protocol_params() {
        let url = "https://x.com/i/api/graphql/abc123/Likes?variables=%7B%22userId%22%3A%221%22%7D&features=%7B%22a%22%3Atrue%7D&fieldToggles=%7B%22b%22%3Afalse%7D";
        let (base, features, fieldtoggles) = split_url_and_extract_protocol(url).unwrap();
        assert_eq!(base, "https://x.com/i/api/graphql/abc123/Likes");
        assert_eq!(features, r#"{"a":true}"#);
        assert_eq!(fieldtoggles, r#"{"b":false}"#);
    }

    #[test]
    fn split_url_rejects_non_likes() {
        let url = "https://x.com/i/api/graphql/abc/HomeTimeline?features=%7B%7D";
        let result = split_url_and_extract_protocol(url);
        assert!(result.is_err());
    }

    #[test]
    fn split_url_allows_missing_fieldtoggles() {
        let url = "https://x.com/i/api/graphql/abc/Likes?variables=%7B%7D&features=%7B%7D";
        let (_, _, fieldtoggles) = split_url_and_extract_protocol(url).unwrap();
        assert!(!fieldtoggles.is_empty());
    }

    #[test]
    fn update_download_sandbox_creates_file_with_only_field() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("private_tokens.env");
        update_download_sandbox_only("/some/dir", &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim(), "DOWNLOAD_SANDBOX_BASE_DIR=/some/dir");
    }

    #[test]
    fn run_setup_from_text_missing_cookie_is_invalid_argument() {
        // 含 Bearer 但无 -b 参数
        let curl = "curl 'https://x.com/i/api/graphql/abc/Likes?features=%7B%7D' -H 'Authorization: Bearer xxx'";
        let result = run_setup_from_text(curl, None);
        let err = result.expect_err("应当因缺 cookie 失败");
        assert_eq!(err.kind, ErrorKind::InvalidArgument, "got: {:?}", err);
    }

    #[test]
    fn run_setup_from_text_missing_bearer_is_invalid_argument() {
        // 含 cookie 但无 Authorization
        let curl = "curl 'https://x.com/i/api/graphql/abc/Likes?features=%7B%7D' -b 'auth_token=x; ct0=y; twid=u%3D1; personalization_id=p'";
        let result = run_setup_from_text(curl, None);
        let err = result.expect_err("应当因缺 Bearer 失败");
        assert_eq!(err.kind, ErrorKind::InvalidArgument, "got: {:?}", err);
    }

    #[test]
    fn run_setup_from_text_non_likes_url_is_invalid_argument() {
        let curl = "curl 'https://x.com/i/api/graphql/abc/HomeTimeline' -H 'Authorization: Bearer xxx' -b 'auth_token=x; ct0=y; twid=u%3D1; personalization_id=p'";
        let result = run_setup_from_text(curl, None);
        let err = result.expect_err("应当因端点不是 Likes 失败");
        assert_eq!(err.kind, ErrorKind::InvalidArgument, "got: {:?}", err);
    }

    #[test]
    fn update_download_sandbox_preserves_other_fields() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("private_tokens.env");
        std::fs::write(
            &path,
            "USER_ID=123\nDOWNLOAD_SANDBOX_BASE_DIR=/old\nAUTH_TOKEN=xxx\n",
        )
        .unwrap();
        update_download_sandbox_only("/new", &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("USER_ID=123"));
        assert!(content.contains("AUTH_TOKEN=xxx"));
        assert!(content.contains("DOWNLOAD_SANDBOX_BASE_DIR=/new"));
        assert!(!content.contains("DOWNLOAD_SANDBOX_BASE_DIR=/old"));
    }
}
