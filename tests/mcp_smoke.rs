//! End-to-end smoke test for `xld serve --mcp`.
//!
//! 目的：在 CI 环境（无 mcp-inspector / 无 X cookies）能跑的纯 stdio 集成测试。
//! 通过 spawn 子进程 + 喂入手写 JSON-RPC 请求 + 解析 stdout，验证：
//! 1. server 启动（initialize 握手）
//! 2. tools/list 返回 4 个预期工具，每个工具有 inputSchema
//! 3. stdin EOF 触发优雅关闭
//!
//! 不验证：tools/call 实际执行（需要凭据），cancellation（v2.0 deferred）。

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
    "auth_status",
    "setup_from_curl",
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

/// V2.0 cancellation 处理：收到 `notifications/cancelled` 后 server 必须**继续存活**
/// （不崩溃、不断开），且后续请求仍能正常响应。这验证了 design D10 "ignore cancellation"
/// 的实施——v2.1 才会真正让 cancellation 生效。
#[tokio::test(flavor = "multi_thread")]
async fn mcp_server_ignores_cancellation_notification() {
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

    // initialize 握手
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

    // 发一个 cancellation notification 给假的 request id（v2.0 应当被忽略）
    stdin
        .write_all(format!("{}\n", jsonrpc_cancelled_notification(99999)).as_bytes())
        .await
        .unwrap();

    // 立刻发 ping，验证 server 仍存活
    stdin
        .write_all(format!("{}\n", jsonrpc_ping(42)).as_bytes())
        .await
        .unwrap();

    // 收 ping response（必须是同一 server 仍在运行——否则 stdout 就 EOF 了）
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

    // 优雅关闭
    drop(stdin);
    let exit_status = timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(exit_status.success());
}
