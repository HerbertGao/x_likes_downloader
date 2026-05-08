use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ErrorKind, ErrorPayload};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEnvelope {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub meta: Meta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

impl OutputEnvelope {
    pub fn success(data: Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            meta: Meta::default(),
            error: None,
        }
    }

    pub fn success_with_meta(data: Value, meta: Meta) -> Self {
        Self {
            ok: true,
            data: Some(data),
            meta,
            error: None,
        }
    }

    pub fn failure(error: ErrorPayload) -> Self {
        Self {
            ok: false,
            data: None,
            meta: Meta::default(),
            error: Some(error),
        }
    }

    pub fn failure_kind(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::failure(ErrorPayload::new(kind, message))
    }

    pub fn to_stdout_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            "{\"ok\":false,\"meta\":{\"schema_version\":1},\"error\":{\"kind\":\"internal_error\",\"message\":\"failed to serialize envelope\"}}".to_string()
        })
    }

    /// Print the envelope to stdout and return the appropriate exit code.
    pub fn emit(&self) -> i32 {
        println!("{}", self.to_stdout_json());
        if self.ok {
            0
        } else {
            self.error.as_ref().map(|e| e.kind.exit_code()).unwrap_or(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_envelope_round_trip() {
        let env = OutputEnvelope::success(json!({"foo": "bar"}));
        let s = env.to_stdout_json();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["data"]["foo"], "bar");
        assert_eq!(parsed["meta"]["schema_version"], 1);
    }

    #[test]
    fn failure_envelope_has_error_kind() {
        let env = OutputEnvelope::failure_kind(ErrorKind::AuthExpired, "cookie expired");
        let s = env.to_stdout_json();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["error"]["kind"], "auth_expired");
    }
}
