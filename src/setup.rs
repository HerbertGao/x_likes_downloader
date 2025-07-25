use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::Path;

#[derive(Parser, Clone)]
#[command(name = "setup")]
#[command(about = "初始化X下载器配置")]
pub struct SetupArgs {
    /// curl命令文件路径
    #[arg(default_value = "curl_command.txt")]
    curl_file: String,
}

#[derive(Debug)]
struct ParsedCurl {
    bearer_token: String,
    cookie_str: String,
    user_agent: Option<String>,
    x_client_uuid: Option<String>,
    x_client_transaction_id: Option<String>,
}

#[derive(Debug)]
struct ParsedCookies {
    twid: String,
    auth_token: String,
    ct0: String,
    personalization_id: String,
}

pub fn run_setup(args: SetupArgs) -> Result<()> {
    // 读取curl命令文件
    let curl_command = fs::read_to_string(&args.curl_file)
        .with_context(|| format!("读取 {} 失败", args.curl_file))?;

    // 解析curl命令
    let parsed = parse_curl_command(&curl_command)?;
    let cookies = parse_cookies(&parsed.cookie_str)?;

    // 保存私有令牌
    save_private_tokens(
        &cookies.twid,
        &parsed.bearer_token,
        &cookies.auth_token,
        &cookies.ct0,
        &cookies.personalization_id,
        parsed.user_agent.as_deref().unwrap_or(""),
        parsed.x_client_uuid.as_deref().unwrap_or(""),
        parsed.x_client_transaction_id.as_deref().unwrap_or(""),
        "data/private_tokens.env",
    )?;

    println!("初始化完成。");
    Ok(())
}

fn parse_curl_command(curl_command: &str) -> Result<ParsedCurl> {
    let header_regex = Regex::new(r#"-H\s+'([^']+)'"#)?;
    let cookie_regex = Regex::new(r#"-b\s+'([^']+)'"#)?;
    let bearer_regex = Regex::new(r#"Bearer\s+(\S+)"#)?;

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

    let bearer_token = bearer_token
        .ok_or_else(|| anyhow::anyhow!("无法解析Bearer Token"))?;

    Ok(ParsedCurl {
        bearer_token,
        cookie_str,
        user_agent,
        x_client_uuid,
        x_client_transaction_id,
    })
}

fn parse_cookies(cookie_str: &str) -> Result<ParsedCookies> {
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

        // 直接使用原始值，不进行URL解码
        match *key {
            "twid" => {
                result.twid = if value.starts_with("u=") {
                    value[2..].to_string()
                } else {
                    value.to_string()
                };
                // 手动处理URL编码的u=前缀
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

fn save_private_tokens(
    user_id: &str,
    bearer_token: &str,
    auth_token: &str,
    ct0: &str,
    personalization_id: &str,
    user_agent: &str,
    x_client_uuid: &str,
    x_client_transaction_id: &str,
    filename: &str,
) -> Result<()> {
    // 确保目录存在
    if let Some(parent) = Path::new(filename).parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format!(
        "USER_ID={}\nBEARER_TOKEN={}\nAUTH_TOKEN={}\nCT0={}\nPERSONALIZATION_ID={}\nUSER_AGENT={}\nX_CLIENT_UUID={}\nX_CLIENT_TRANSACTION_ID={}\n",
        user_id, bearer_token, auth_token, ct0, personalization_id, user_agent, x_client_uuid, x_client_transaction_id
    );

    fs::write(filename, content)?;
    println!("生成 {} 成功！", filename);
    Ok(())
} 