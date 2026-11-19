//! End-to-end smoke test for `xld serve --mcp`.
//!
//! 目的：在 CI 环境（无 mcp-inspector / 无 X cookies）能跑的纯 stdio 集成测试。
//! 通过 spawn 子进程 + 喂入手写 JSON-RPC 请求 + 解析 stdout，验证：
//! 1. server 启动（initialize 握手）
//! 2. tools/list 返回 6 个预期工具，每个工具有 inputSchema
//! 3. tools/call `user_tweets` 非法 username 返回 isError + `invalid_argument` 信封（免凭据免网络）
//! 4. stdin EOF 触发优雅关闭
//!
//! 不验证：tools/call 需要凭据的正常执行，cancellation（v2.0 deferred）。

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const BINARY_PATH: &str = env!("CARGO_BIN_EXE_x_likes_downloader");

const EXPECTED_TOOLS: &[&str] = &[
    "list_likes",
    "download_media",
    "commit_download_record",
    "get_download_record_batch",
    "auth_status",
    "setup_from_curl",
    "fetch_tweet",
    "user_tweets",
];

fn jsonrpc_initialize() -> String {
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"xld-smoke-test","version":"0.0.1"}}}"#.to_string()
}

fn jsonrpc_initialized_notification() -> String {
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string()
}

fn jsonrpc_tools_list(id: u64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#)
}

fn jsonrpc_ping(id: u64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#)
}

fn jsonrpc_cancelled_notification(request_id: u64) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"notifications/cancelled","params":{{"requestId":{request_id},"reason":"smoke-test"}}}}"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_server_handshake_and_tools_list() {
    let mut child = Command::new(BINARY_PATH)
        .args(["serve", "--mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn xld serve --mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout).lines();

    // 发 initialize
    stdin
        .write_all(format!("{}\n", jsonrpc_initialize()).as_bytes())
        .await
        .expect("write initialize");

    // 收 initialize response
    let init_response = timeout(Duration::from_secs(5), reader.next_line())
        .await
        .expect("initialize timeout")
        .expect("read initialize response")
        .expect("expected response line");
    let init_json: Value =
        serde_json::from_str(&init_response).expect("parse initialize response JSON");
    assert_eq!(init_json["jsonrpc"], "2.0");
    assert_eq!(init_json["id"], 1);
    assert!(
        init_json.get("result").is_some(),
        "initialize must succeed; got: {init_json:?}"
    );
    let instructions = init_json["result"]["instructions"]
        .as_str()
        .expect("server instructions");
    assert!(
        instructions.contains("DOWNLOAD_RECORD") && instructions.contains("原生确认卡"),
        "instructions must describe the two-stage record workflow: {instructions}"
    );

    // serverInfo 必须是本 crate 而不是 rmcp 默认值（防止 from_build_env 拿到 rmcp 自己的 name）
    let server_info = &init_json["result"]["serverInfo"];
    assert_eq!(
        server_info["name"], "x_likes_downloader",
        "serverInfo.name must be the application crate, not rmcp; got: {server_info:?}"
    );
    assert_eq!(
        server_info["version"],
        env!("CARGO_PKG_VERSION"),
        "serverInfo.version must match crate version; got: {server_info:?}"
    );

    // 发 initialized notification + tools/list
    stdin
        .write_all(format!("{}\n", jsonrpc_initialized_notification()).as_bytes())
        .await
        .expect("write initialized notif");
    stdin
        .write_all(format!("{}\n", jsonrpc_tools_list(2)).as_bytes())
        .await
        .expect("write tools/list");

    // 收 tools/list response（可能要跳过非响应消息，但通常直接返回）
    let tools_response_line = loop {
        let line = timeout(Duration::from_secs(5), reader.next_line())
            .await
            .expect("tools/list timeout")
            .expect("read tools/list response")
            .expect("expected response line");
        let v: Value = serde_json::from_str(&line).expect("parse line as JSON");
        if v.get("id") == Some(&Value::from(2u64)) {
            break v;
        }
    };

    // 解析工具列表
    let tools = tools_response_line["result"]["tools"]
        .as_array()
        .expect("tools/list result must contain tools array");
    assert_eq!(
        tools.len(),
        EXPECTED_TOOLS.len(),
        "expected exactly {} tools, got {}: {:?}",
        EXPECTED_TOOLS.len(),
        tools.len(),
        tools.iter().map(|t| t["name"].clone()).collect::<Vec<_>>()
    );

    let names: std::collections::HashSet<String> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();
    for expected in EXPECTED_TOOLS {
        assert!(
            names.contains(*expected),
            "missing expected tool '{expected}' in {names:?}"
        );
    }

    // 每个工具必须有 inputSchema
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or("?");
        assert!(
            tool.get("inputSchema").is_some(),
            "tool '{name}' missing inputSchema"
        );
        assert!(
            tool["description"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "tool '{name}' missing description"
        );
    }

    let by_name = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    let download_desc = by_name("download_media")["description"].as_str().unwrap();
    assert!(download_desc.contains("DOWNLOAD_RECORD"));
    assert!(download_desc.contains("commit_download_record"));
    assert!(download_desc.contains("confirmed:true"));
    assert!(by_name("download_media").get("outputSchema").is_some());
    let commit_desc = by_name("commit_download_record")["description"]
        .as_str()
        .unwrap();
    assert!(commit_desc.contains("不重新下载") || commit_desc.contains("不访问网络"));
    assert!(commit_desc.contains("confirmed"));
    assert!(by_name("commit_download_record")
        .get("outputSchema")
        .is_some());
    assert!(by_name("get_download_record_batch")
        .get("outputSchema")
        .is_some());

    // tools/call user_tweets：非法 username 走免凭据免网络的 hermetic 错误路径，
    // 钉住新工具的 rmcp 参数反序列化（Parameters<UserTweetsRequest>）与业务错误信封 wiring。
    stdin
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"user_tweets\",\"arguments\":{\"username\":\"a b\"}}}\n",
        )
        .await
        .expect("write tools/call user_tweets");

    let call_response_line = loop {
        let line = timeout(Duration::from_secs(5), reader.next_line())
            .await
            .expect("tools/call timeout")
            .expect("read tools/call response")
            .expect("expected response line");
        let v: Value = serde_json::from_str(&line).expect("parse line as JSON");
        if v.get("id") == Some(&Value::from(3u64)) {
            break v;
        }
    };
    assert!(
        call_response_line["result"]["isError"]
            .as_bool()
            .unwrap_or(false),
        "user_tweets invalid-username call must return isError: true, got: {call_response_line:?}"
    );
    let err_text = call_response_line["result"]["content"][0]["text"]
        .as_str()
        .expect("error content text");
    assert!(
        err_text.contains("\"kind\":\"invalid_argument\""),
        "error envelope kind must be invalid_argument, got: {err_text}"
    );

    // stdin EOF 优雅关闭
    drop(stdin);
    let exit_status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("server should shut down within 5s after stdin EOF")
        .expect("await server exit");
    assert!(
        exit_status.success(),
        "server should exit 0 after stdin EOF, got: {exit_status:?}"
    );
}

/// V2.1: server 收到 `notifications/cancelled` 后必须**继续存活**（即使针对 unknown
/// request id），且后续请求仍能正常响应。rmcp 1.6 的 `local_ct_pool` 在
/// request id 未注册时静默忽略 cancel；server 的 `on_cancelled` 钩子仍被调用，
/// stderr 写一行诊断日志。
///
/// （真实生效的 cancellation 由 `tests/cancellation_smoke.rs` 覆盖——需 wiremock。）
#[tokio::test(flavor = "multi_thread")]
async fn mcp_server_handles_cancellation_for_unknown_request() {
    let mut child = Command::new(BINARY_PATH)
        .args(["serve", "--mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn xld serve --mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout).lines();

    stdin
        .write_all(format!("{}\n", jsonrpc_initialize()).as_bytes())
        .await
        .unwrap();
    let _init = timeout(Duration::from_secs(5), reader.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("init resp");

    stdin
        .write_all(format!("{}\n", jsonrpc_initialized_notification()).as_bytes())
        .await
        .unwrap();

    // 发 cancellation notification 给一个未注册的 request id；rmcp 内部静默忽略
    stdin
        .write_all(format!("{}\n", jsonrpc_cancelled_notification(99999)).as_bytes())
        .await
        .unwrap();

    // 立刻发 ping，验证 server 仍存活
    stdin
        .write_all(format!("{}\n", jsonrpc_ping(42)).as_bytes())
        .await
        .unwrap();

    let ping_response = loop {
        let line = timeout(Duration::from_secs(5), reader.next_line())
            .await
            .expect("ping timeout (server may have died after cancellation)")
            .expect("read ping response")
            .expect("expected response line");
        let v: Value = serde_json::from_str(&line).expect("parse line as JSON");
        if v.get("id") == Some(&Value::from(42u64)) {
            break v;
        }
    };
    assert_eq!(ping_response["jsonrpc"], "2.0");
    assert!(
        ping_response.get("result").is_some(),
        "ping should succeed after cancellation; got: {ping_response:?}"
    );

    drop(stdin);
    let exit_status = timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(exit_status.success());
}

/// 非法 batch_id 与空的待确认列表都不得创建 DOWNLOAD_RECORD。全程使用临时目录。
#[tokio::test(flavor = "multi_thread")]
async fn mcp_record_tools_do_not_write_on_refusal_or_invalid_id() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = dir.path().join("downloaded_tweet_ids.txt");
    let credentials = dir.path().join("no-such-tokens.env");
    let mut child = Command::new(BINARY_PATH)
        .args(["serve", "--mcp"])
        .env("DOWNLOAD_RECORD", &record)
        .env("XLD_CREDENTIALS_FILE", &credentials)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    stdin
        .write_all(format!("{}\n", jsonrpc_initialize()).as_bytes())
        .await
        .unwrap();
    let _ = timeout(Duration::from_secs(5), reader.next_line())
        .await
        .unwrap()
        .unwrap();
    stdin
        .write_all(format!("{}\n", jsonrpc_initialized_notification()).as_bytes())
        .await
        .unwrap();
    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"commit_download_record","arguments":{"batch_id":"../outside/batch.json","tweet_ids":["1"]}}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    let commit_response = next_response(&mut reader, 7).await;
    assert!(commit_response["result"]["isError"]
        .as_bool()
        .unwrap_or(false));
    let text = commit_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("invalid_argument"), "{text}");

    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_download_record_batch","arguments":{}}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    let listed = next_response(&mut reader, 8).await;
    assert!(
        !listed["result"]["isError"].as_bool().unwrap_or(false),
        "{listed:?}"
    );
    let listed_text = listed["result"]["content"][0]["text"].as_str().unwrap();
    let listed_json: Value = serde_json::from_str(listed_text).unwrap();
    assert_eq!(listed_json["pending"].as_array().unwrap().len(), 0);
    assert_eq!(listed_json["user_permission_verified"], false);
    assert!(listed["result"]["structuredContent"].is_object());
    assert!(!record.exists());

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
}

async fn next_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
) -> Value {
    loop {
        let line = timeout(Duration::from_secs(5), reader.next_line())
            .await
            .expect("response timeout")
            .expect("read response")
            .expect("response line");
        let value: Value = serde_json::from_str(&line).expect("json");
        if value.get("id") == Some(&Value::from(id)) {
            return value;
        }
    }
}
