use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthExpired,
    EndpointStale,
    RateLimited,
    NetworkError,
    NotConfigured,
    InternalError,
    SandboxViolation,
    BinaryMissing,
    InvalidItem,
    InvalidArgument,
}

impl ErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::AuthExpired
            | ErrorKind::EndpointStale
            | ErrorKind::RateLimited
            | ErrorKind::NetworkError => 2,
            ErrorKind::NotConfigured
            | ErrorKind::InternalError
            | ErrorKind::SandboxViolation
            | ErrorKind::BinaryMissing
            | ErrorKind::InvalidItem
            | ErrorKind::InvalidArgument => 1,
        }
    }

    pub fn default_hint(self) -> &'static str {
        match self {
            ErrorKind::AuthExpired => "凭据已失效，请重新从浏览器导出 cURL 并运行 xld setup",
            ErrorKind::EndpointStale => {
                "X GraphQL 端点已变更，请重新从浏览器导出 cURL 并运行 xld setup"
            }
            ErrorKind::RateLimited => "请求被 X 限流，稍后再试",
            ErrorKind::NetworkError => "网络异常，请检查连接后重试",
            ErrorKind::NotConfigured => "本地尚未配置，请先运行 xld setup",
            ErrorKind::InternalError => "内部错误，请提交 issue 附 stderr 输出",
            ErrorKind::SandboxViolation => "下载子目录违反沙箱规则",
            ErrorKind::BinaryMissing => "未找到 xld 可执行文件，请从 GitHub Releases 安装",
            ErrorKind::InvalidItem => "media item 字段不完整或格式错误",
            ErrorKind::InvalidArgument => "命令行参数不合法",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ErrorPayload {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: Some(kind.default_hint().to_string()),
            retry_after: None,
        }
    }

    pub fn with_retry_after(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs);
        self
    }
}

/// Maps an HTTP status code from X to a structured ErrorKind.
/// Pure function — easily unit-tested.
pub fn classify_status(status: u16) -> Option<ErrorKind> {
    match status {
        200..=299 => None,
        401 | 403 => Some(ErrorKind::AuthExpired),
        404 | 410 => Some(ErrorKind::EndpointStale),
        429 => Some(ErrorKind::RateLimited),
        500..=599 => Some(ErrorKind::NetworkError),
        _ => Some(ErrorKind::InternalError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_success_returns_none() {
        assert_eq!(classify_status(200), None);
        assert_eq!(classify_status(204), None);
    }

    #[test]
    fn classify_auth_expired() {
        assert_eq!(classify_status(401), Some(ErrorKind::AuthExpired));
        assert_eq!(classify_status(403), Some(ErrorKind::AuthExpired));
    }

    #[test]
    fn classify_endpoint_stale() {
        assert_eq!(classify_status(404), Some(ErrorKind::EndpointStale));
        assert_eq!(classify_status(410), Some(ErrorKind::EndpointStale));
    }

    #[test]
    fn classify_rate_limited() {
        assert_eq!(classify_status(429), Some(ErrorKind::RateLimited));
    }

    #[test]
    fn classify_5xx_is_network_error_not_internal() {
        assert_eq!(classify_status(500), Some(ErrorKind::NetworkError));
        assert_eq!(classify_status(502), Some(ErrorKind::NetworkError));
        assert_eq!(classify_status(503), Some(ErrorKind::NetworkError));
    }

    #[test]
    fn classify_unknown_status_falls_to_internal() {
        assert_eq!(classify_status(451), Some(ErrorKind::InternalError));
        assert_eq!(classify_status(300), Some(ErrorKind::InternalError));
    }

    #[test]
    fn exit_codes_match_spec() {
        assert_eq!(ErrorKind::AuthExpired.exit_code(), 2);
        assert_eq!(ErrorKind::EndpointStale.exit_code(), 2);
        assert_eq!(ErrorKind::RateLimited.exit_code(), 2);
        assert_eq!(ErrorKind::NetworkError.exit_code(), 2);
        assert_eq!(ErrorKind::NotConfigured.exit_code(), 1);
        assert_eq!(ErrorKind::InternalError.exit_code(), 1);
        assert_eq!(ErrorKind::SandboxViolation.exit_code(), 1);
    }

    #[test]
    fn error_kind_serializes_snake_case() {
        let json = serde_json::to_string(&ErrorKind::AuthExpired).unwrap();
        assert_eq!(json, "\"auth_expired\"");
        let json = serde_json::to_string(&ErrorKind::EndpointStale).unwrap();
        assert_eq!(json, "\"endpoint_stale\"");
    }
}
