//! Wire format of the control socket: one JSON object per line, requests
//! `{id, method, params}` and responses `{id, result}` or `{id, error}`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped when a method changes shape. Adding methods does not bump it.
pub const PROTOCOL_VERSION: u32 = 1;

/// Longest request or response line the socket accepts.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(ErrorBody {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}
