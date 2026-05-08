//! MCP server (v2 Agent 接口).
//!
//! 用 [`rmcp`](https://docs.rs/rmcp) 把 4 个 lib 函数包成 MCP 工具：
//! - `list_likes`      ← `agent::list_likes`
//! - `download_media`  ← `agent::download_media`
//! - `auth_status`     ← `agent::auth_status`
//! - `setup_from_curl` ← `agent::import_curl`
//!
//! Transport：仅 stdio。HTTP / SSE 在 v2.0 不支持（凭据本地化原则）。
//!
//! Cancellation：v2.0 收到 MCP `notifications/cancelled` 时仅记录到 stderr 诊断日志，
//! **不**中断正在跑的工具。参见 design D10。

use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, CancelledNotificationParam, Content, ErrorData as McpError, Meta,
    ProgressNotificationParam, ProgressToken,
};
use rmcp::service::NotificationContext;
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{DownloadOpts, ListOpts, MediaItem, NullSink, ProgressEvent, ProgressSink};
use super::{auth_status, download_media, import_curl, list_likes};
use crate::error::ErrorPayload;

// ============================================================================
// 请求类型（带 JsonSchema 派生，rmcp 自动用它们生成工具的 inputSchema）
// ============================================================================

/// `list_likes` 工具的输入参数。
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListLikesRequest {
    /// 拉取全部历史点赞（按游标翻页直至无更多数据）。默认 false（仅首页）。
    #[serde(default)]
    pub all: bool,
    /// 从指定游标继续拉取（用于增量同步）。
    #[serde(default)]
    pub since_cursor: Option<String>,
    /// 单页条数。默认沿用配置值（通常 20）。
    #[serde(default)]
    pub count: Option<u32>,
    /// 同时返回原始 GraphQL entry。**显著增加 token 成本**（5–10 倍），仅供调试。
    #[serde(default)]
    pub include_raw: bool,
}

/// `download_media` 工具的输入参数。
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct DownloadMediaRequest {
    /// 要下载的 MediaItem 数组（直接来自 `list_likes` 输出的 `tweets[].media[]`）。
    pub items: Vec<MediaItem>,
    /// 沙箱内子目录名。不允许 `..` / 绝对路径 / Windows 盘符。
    #[serde(default)]
    pub subdir: Option<String>,
    /// 并发下载数。范围 [1, 16]，默认 4。
    #[serde(default)]
    pub concurrency: Option<u32>,
}

/// `setup_from_curl` 工具的输入参数。
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetupFromCurlRequest {
    /// 完整的 cURL 文本（从浏览器 DevTools "Copy as cURL (bash)" 拷贝）。
    pub curl_text: String,
}

// ============================================================================
// MCP Server 主体
// ============================================================================

/// MCP server 实例。无状态——每次工具调用都重读 `Config`（凭据热加载）。
#[derive(Debug, Clone)]
pub struct XldMcpServer {
    tool_router: ToolRouter<Self>,
}

impl XldMcpServer {
    /// 创建新 server 实例并初始化 tool router。
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for XldMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl XldMcpServer {
    /// 拉取当前账号的点赞推文列表（扁平 schema：id / author_handle / text / media[] / created_at 等）。
    /// 返回值含 cursor 用于增量同步。Agent 应当把 `tweets[].media[]` 元素直接传给 `download_media` 下载。
    #[tool(
        name = "list_likes",
        description = "拉取当前 X 账号的点赞推文列表，扁平 schema 含 id / author_handle / text / media[] / created_at / tweet_url 等字段。可通过 `count` 限制条数、`all=true` 翻全部历史、`since_cursor` 做增量同步。include_raw=true 时附带原始 GraphQL entry（token 成本高，仅调试用）。"
    )]
    async fn list_likes(
        &self,
        Parameters(req): Parameters<ListLikesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let opts = ListOpts {
            all: req.all,
            since_cursor: req.since_cursor,
            count: req.count,
            include_raw: req.include_raw,
        };
        match list_likes(opts).await {
            Ok(out) => Ok(success_call_result(&out)),
            Err(payload) => Ok(error_call_result(&payload)),
        }
    }

    /// 按 MediaItem 数组下载媒体到沙箱目录。
    #[tool(
        name = "download_media",
        description = "把一组 MediaItem（来自 list_likes 输出的 tweets[].media[]）下载到本机沙箱目录。文件名格式 `{author_handle}_{tweet_id}_{suggested_filename}`，文件 mtime 设为推文发布时间。`subdir` 可指定沙箱内子目录（受路径穿越校验）。`concurrency` 钳位 [1,16]，默认 4。Agent 进度通过 MCP notifications/progress 推送（仅当请求带 progressToken）。"
    )]
    async fn download_media(
        &self,
        Parameters(req): Parameters<DownloadMediaRequest>,
        meta: Meta,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let opts = DownloadOpts {
            subdir: req.subdir,
            concurrency: req.concurrency.unwrap_or(4),
            ..Default::default()
        };

        // 按 progressToken 注入 sink。McpProgressSink 强类型保留以便 flush()。
        let mcp_sink = meta
            .get_progress_token()
            .map(|token| Arc::new(McpProgressSink::new(token, peer, req.items.len())));

        let sink: Arc<dyn ProgressSink> = match &mcp_sink {
            Some(s) => s.clone() as Arc<dyn ProgressSink>,
            None => Arc::new(NullSink),
        };

        let result = download_media(&req.items, &opts, sink).await;

        // 关键：tool 返回前 flush 所有 pending progress notifications，
        // 否则 client 可能在收到 response 后立即关闭 stdin 丢弃通知。
        if let Some(s) = mcp_sink {
            s.flush().await;
        }

        match result {
            Ok(out) => {
                // 镜像 CLI 端 `run_media_download` 的"全失败"逻辑：
                // total > 0 且无任何 downloaded/skipped → MCP isError=true，
                // 但 content 仍带完整 DownloadOutput 让 Agent 看到 per-item 失败原因。
                let total = out.summary.total;
                let any_success = out.summary.downloaded > 0 || out.summary.skipped > 0;
                if total > 0 && !any_success {
                    let payload = ErrorPayload::new(
                        crate::error::ErrorKind::NetworkError,
                        format!("全部 {} 个 media item 下载失败", total),
                    );
                    return Ok(all_failed_call_result(&out, &payload));
                }
                Ok(success_call_result(&out))
            }
            Err(payload) => Ok(error_call_result(&payload)),
        }
    }

    /// 探测当前凭据是否仍可访问 X Likes 端点（轻量真实请求，约 200ms）。
    #[tool(
        name = "auth_status",
        description = "探测当前本地凭据是否仍可访问 X GraphQL Likes 端点。发起一次 count=1 的轻量真实请求（约 200ms）。返回 status: healthy / auth_expired / endpoint_stale / rate_limited / network_error / not_configured。Agent 仅应在 (1) 会话起始预检、(2) 其它工具返回 auth_expired/endpoint_stale 后确认、(3) 用户显式询问 时调用，不应循环或预检式调用。"
    )]
    async fn auth_status(&self) -> Result<CallToolResult, McpError> {
        let status = auth_status().await;
        match status {
            crate::agent::types::AuthStatus::Healthy { ref checked_at } => {
                Ok(success_call_result(&serde_json::json!({
                    "status": "healthy",
                    "checked_at": checked_at,
                })))
            }
            crate::agent::types::AuthStatus::AuthExpired => Ok(error_call_result(
                &ErrorPayload::new(crate::error::ErrorKind::AuthExpired, "凭据失效"),
            )),
            crate::agent::types::AuthStatus::EndpointStale => {
                Ok(error_call_result(&ErrorPayload::new(
                    crate::error::ErrorKind::EndpointStale,
                    "X GraphQL 端点已变更",
                )))
            }
            crate::agent::types::AuthStatus::RateLimited { retry_after } => {
                let mut payload = ErrorPayload::new(crate::error::ErrorKind::RateLimited, "被限流");
                if let Some(ra) = retry_after {
                    payload = payload.with_retry_after(ra);
                }
                Ok(error_call_result(&payload))
            }
            crate::agent::types::AuthStatus::NetworkError { message } => Ok(error_call_result(
                &ErrorPayload::new(crate::error::ErrorKind::NetworkError, message),
            )),
            crate::agent::types::AuthStatus::NotConfigured => Ok(error_call_result(
                &ErrorPayload::new(crate::error::ErrorKind::NotConfigured, "未导入凭据"),
            )),
        }
    }

    /// 从用户提供的 cURL 文本导入凭据 + 协议参数到本地（首次配置或 cookie 失效后重导）。
    #[tool(
        name = "setup_from_curl",
        description = "从用户提供的浏览器 cURL 文本导入 X 凭据（cookies / bearer / 协议参数）到本地稳定路径。安全：函数不写临时文件，整个解析在内存中完成；返回值不回显任何凭据。Agent 应在 auth_expired 或 endpoint_stale 后引导用户提供新 cURL 调用此工具。"
    )]
    async fn setup_from_curl(
        &self,
        Parameters(req): Parameters<SetupFromCurlRequest>,
    ) -> Result<CallToolResult, McpError> {
        // import_curl 是同步函数（内存解析+写文件，无网络）；spawn_blocking 不必要
        match import_curl(&req.curl_text) {
            Ok(out) => Ok(success_call_result(&out)),
            Err(payload) => Ok(error_call_result(&payload)),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for XldMcpServer {
    /// V2.0: 收到 cancellation 通知时仅记录到 stderr 诊断日志，**不**中断正在跑的工具、
    /// **不**清理已写入磁盘的下载文件。客户端如需强制终止可关闭 stdin（走 EOF 优雅关闭路径）。
    /// 真实 cancellation 响应推迟到 v2.1。
    async fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        let reason = notification.reason.as_deref().unwrap_or("(no reason)");
        eprintln!(
            "xld serve --mcp: received cancellation notification for request {:?} (reason: {}); ignoring (v2.0 limitation, see design D10)",
            notification.request_id, reason
        );
    }
}

// ============================================================================
// McpProgressSink —— 把 ProgressEvent 映射到 MCP notifications/progress
// ============================================================================

/// `ProgressSink` 实现：把 lib 层的 `ProgressEvent` 转为 MCP `ProgressNotificationParam`
/// 通过 `Peer::notify_progress` 异步发送给客户端。
///
/// 仅在 MCP 请求带 progressToken 时使用；无 token 路径用 `NullSink`。
///
/// **顺序保证**：内部用 `mpsc::UnboundedSender` + 单后台 worker task 串行化。
/// 多并发下载产生的多个 emit() 调用通过 mpsc FIFO 排队，worker 顺序 `await notify_progress`，
/// 保证 client 收到的 notification 顺序与 emit 顺序一致。
///
/// **Flush 保证**：tool handler 必须在返回前调用 [`flush`](Self::flush)，等 worker 把所有
/// in-flight notification 发送完。否则 tool response 可能跟 final notification 竞争，
/// 让 client 在收到 response 后立即关闭 stdin 时丢失 notification。
pub struct McpProgressSink {
    progress_token: ProgressToken,
    /// `Option<>` 包装方便 `flush()` 时 take 出 sender 让 channel close → worker 退出。
    sender: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<ProgressNotificationParam>>>,
    /// Worker JoinHandle：`flush()` await 它确保所有 pending 都已发出。
    worker: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 记录 batch 总 item 数，与 progress 字段配对作为 total 字段。
    total_items: usize,
    /// 跑了多少 item（item_done 时递增）。
    items_done: std::sync::Mutex<usize>,
}

impl McpProgressSink {
    pub fn new(progress_token: ProgressToken, peer: Peer<RoleServer>, total_items: usize) -> Self {
        let (sender, mut receiver) =
            tokio::sync::mpsc::unbounded_channel::<ProgressNotificationParam>();
        // 单 worker 串行处理 notification：任何错误（transport 已关闭等）静默忽略。
        let worker = tokio::spawn(async move {
            while let Some(notif) = receiver.recv().await {
                let _ = peer.notify_progress(notif).await;
            }
        });
        Self {
            progress_token,
            sender: std::sync::Mutex::new(Some(sender)),
            worker: std::sync::Mutex::new(Some(worker)),
            total_items,
            items_done: std::sync::Mutex::new(0),
        }
    }

    /// 把 progress notification 入队（同步操作，不阻塞）。
    /// 入队失败（worker 已退出 / channel closed）静默忽略。
    fn dispatch(&self, progress: f64, total: Option<f64>, message: Option<String>) {
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            let _ = sender.send(ProgressNotificationParam {
                progress_token: self.progress_token.clone(),
                progress,
                total,
                message,
            });
        }
    }

    /// 关闭入队 channel 并 await worker drain 所有 pending notification。
    /// **必须在 tool handler 返回前调用**，否则 final progress 通知可能晚于
    /// tool response 到达 client（client 收到 response 关闭 stdin 后被丢弃）。
    ///
    /// 多次调用安全：第二次起为 no-op。
    pub async fn flush(&self) {
        // 1. drop sender → channel close → worker 的 receiver.recv() 返回 None → loop 退出
        drop(self.sender.lock().unwrap().take());

        // 2. await worker 完成
        let handle = self.worker.lock().unwrap().take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }
}

impl ProgressSink for McpProgressSink {
    fn emit(&self, event: ProgressEvent) {
        let total_items_f = self.total_items as f64;
        match event {
            ProgressEvent::DownloadStarted { total, concurrency } => {
                self.dispatch(
                    0.0,
                    Some(total as f64),
                    Some(format!(
                        "starting {} items, concurrency={}",
                        total, concurrency
                    )),
                );
            }
            ProgressEvent::ItemStarted { tweet_id, .. } => {
                // 关键：用 items_done 而非 index 作为 progress；
                // 多并发时多个 ItemStarted 会先后触发但 items_done 不变，保单调。
                let items_done = *self.items_done.lock().unwrap();
                self.dispatch(
                    items_done as f64,
                    Some(total_items_f),
                    Some(format!("starting tweet {}", tweet_id)),
                );
            }
            ProgressEvent::ItemProgress {
                tweet_id,
                bytes_done,
                bytes_total,
            } => {
                // 字节级进度只反映在 message；progress 字段仍是 items_done，保单调。
                // 多并发时若用 items_done + item_fraction，不同 in-flight item 间会回退。
                let items_done = *self.items_done.lock().unwrap();
                let bytes_msg = match bytes_total {
                    Some(t) if t > 0 => format!("{}/{}", bytes_done, t),
                    _ => bytes_done.to_string(),
                };
                self.dispatch(
                    items_done as f64,
                    Some(total_items_f),
                    Some(format!("tweet {} {}", tweet_id, bytes_msg)),
                );
            }
            ProgressEvent::ItemDone {
                tweet_id, status, ..
            } => {
                let mut items_done = self.items_done.lock().unwrap();
                *items_done += 1;
                self.dispatch(
                    *items_done as f64,
                    Some(total_items_f),
                    Some(format!("tweet {} {:?}", tweet_id, status)),
                );
            }
            ProgressEvent::DownloadFinished { summary } => {
                self.dispatch(
                    total_items_f,
                    Some(total_items_f),
                    Some(format!(
                        "done: downloaded={} skipped={} failed={}",
                        summary.downloaded, summary.skipped, summary.failed
                    )),
                );
            }
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 构造成功的 `CallToolResult`：把 `data` 序列化为 JSON 文本作为 content。
fn success_call_result<T: Serialize>(data: &T) -> CallToolResult {
    let text = serde_json::to_string(data)
        .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e));
    CallToolResult::success(vec![Content::text(text)])
}

/// 构造错误的 `CallToolResult`（isError=true）：把 `payload` 序列化为 JSON 文本作为 content。
fn error_call_result(payload: &ErrorPayload) -> CallToolResult {
    let text = serde_json::to_string(payload)
        .unwrap_or_else(|_| "{\"kind\":\"internal_error\",\"message\":\"unknown\"}".to_string());
    CallToolResult::error(vec![Content::text(text)])
}

/// 全失败下载场景（download_media 返回 Ok 但 summary 全 failed）：
/// 用 isError=true 表达"操作失败"，但 content 同时携带 ErrorPayload 的 kind/message/hint
/// **以及完整 DownloadOutput**，Agent 能看到每个 item 的具体失败 kind。
fn all_failed_call_result<T: Serialize>(data: &T, payload: &ErrorPayload) -> CallToolResult {
    let combined = serde_json::json!({
        "kind": payload.kind,
        "message": payload.message,
        "hint": payload.hint,
        "retry_after": payload.retry_after,
        "data": data,
    });
    let text = serde_json::to_string(&combined).unwrap_or_else(|_| {
        "{\"kind\":\"internal_error\",\"message\":\"serialize failed\"}".to_string()
    });
    CallToolResult::error(vec![Content::text(text)])
}

// ============================================================================
// 入口
// ============================================================================

/// 启动 MCP server，使用 stdio transport。会一直运行直到 stdin EOF。
pub async fn run_serve_mcp() -> Result<()> {
    eprintln!("xld serve --mcp: starting (stdio transport)");
    let server = XldMcpServer::new();
    let svc = server
        .serve(stdio())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server start failed: {}", e))?;
    eprintln!("xld serve --mcp: connected, awaiting requests");
    svc.waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server runtime error: {}", e))?;
    eprintln!("xld serve --mcp: client disconnected, shutting down");
    Ok(())
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_constructs() {
        let _ = XldMcpServer::new();
        let _ = XldMcpServer::default();
    }

    #[test]
    fn list_likes_request_schema_has_expected_fields() {
        let schema = schemars::schema_for!(ListLikesRequest);
        let json = serde_json::to_value(&schema).unwrap();
        let props = json
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema should have properties");
        assert!(props.contains_key("all"));
        assert!(props.contains_key("since_cursor"));
        assert!(props.contains_key("count"));
        assert!(props.contains_key("include_raw"));
    }

    #[test]
    fn download_media_request_schema_has_expected_fields() {
        let schema = schemars::schema_for!(DownloadMediaRequest);
        let json = serde_json::to_value(&schema).unwrap();
        let props = json
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema should have properties");
        assert!(props.contains_key("items"));
        assert!(props.contains_key("subdir"));
        assert!(props.contains_key("concurrency"));
    }

    #[test]
    fn setup_from_curl_request_schema_has_curl_text_required() {
        let schema = schemars::schema_for!(SetupFromCurlRequest);
        let json = serde_json::to_value(&schema).unwrap();
        let required = json
            .get("required")
            .and_then(|r| r.as_array())
            .expect("schema should have required");
        assert!(required.iter().any(|v| v == "curl_text"));
    }

    #[test]
    fn success_call_result_has_text_content() {
        let result = success_call_result(&serde_json::json!({"foo": "bar"}));
        assert_eq!(result.is_error, Some(false));
        assert!(!result.content.is_empty());
    }

    #[test]
    fn error_call_result_has_iserror_true() {
        let payload = ErrorPayload::new(crate::error::ErrorKind::NotConfigured, "test");
        let result = error_call_result(&payload);
        assert_eq!(result.is_error, Some(true));
    }
}
