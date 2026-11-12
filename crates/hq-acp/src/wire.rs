//! Newline-delimited JSON-RPC as ACP agents speak it. Parsing is tolerant on purpose: agents differ
//! in field names and extras (see docs/plans/acp-lane.md), so everything stays a `Value` until a
//! typed view picks out the few fields HQ needs.

use serde_json::{Value, json};

const JSONRPC: &str = "2.0";
pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a JSON-RPC message: {0}")]
    Shape(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The answer to a request this side sent.
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
    /// The agent asks this side for something; `id` is echoed back verbatim.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn parse_line(line: &str) -> Result<Incoming, WireError> {
    let v: Value = serde_json::from_str(line)?;
    let method = v.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match (method, v.get("id")) {
        (Some(method), Some(id)) => Ok(Incoming::Request {
            id: id.clone(),
            method,
            params,
        }),
        (Some(method), None) => Ok(Incoming::Notification { method, params }),
        (None, Some(id)) => response(&v, id),
        (None, None) => Err(WireError::Shape("neither method nor id")),
    }
}

fn response(v: &Value, id: &Value) -> Result<Incoming, WireError> {
    // Ids are ours and numeric, but an agent may hand one back quoted.
    let numeric = id
        .as_u64()
        .or_else(|| id.as_str().and_then(|s| s.parse().ok()));
    let id = numeric.ok_or(WireError::Shape("response id is not a number"))?;
    if let Some(e) = v.get("error") {
        let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        return Ok(Incoming::Response {
            id,
            result: Err(RpcError { code, message }),
        });
    }
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    Ok(Incoming::Response {
        id,
        result: Ok(result),
    })
}

pub fn request(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": JSONRPC, "id": id, "method": method, "params": params}).to_string()
}

pub fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc": JSONRPC, "method": method, "params": params}).to_string()
}

pub fn reply_ok(id: &Value, result: Value) -> String {
    json!({"jsonrpc": JSONRPC, "id": id, "result": result}).to_string()
}

pub fn reply_error(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": JSONRPC, "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_three_shapes_apart() {
        let req = parse_line(
            r#"{"jsonrpc":"2.0","id":"a1","method":"session/request_permission","params":{}}"#,
        );
        assert!(matches!(req, Ok(Incoming::Request { .. })));
        let note = parse_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#);
        assert!(matches!(note, Ok(Incoming::Notification { .. })));
        let resp = parse_line(r#"{"id":3,"result":{"stopReason":"end_turn"},"jsonrpc":"2.0"}"#);
        assert!(matches!(
            resp,
            Ok(Incoming::Response {
                id: 3,
                result: Ok(_)
            })
        ));
    }

    #[test]
    fn an_error_response_keeps_code_and_message() {
        let msg = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}"#;
        match parse_line(msg) {
            Ok(Incoming::Response {
                id: 2,
                result: Err(e),
            }) => {
                assert_eq!(e.code, -32000);
                assert_eq!(e.message, "Authentication required");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_line("not json").is_err());
        assert!(parse_line("{}").is_err());
        assert!(parse_line(r#"{"id":"x","result":1}"#).is_err());
        let quoted = parse_line(r#"{"id":"4","result":1}"#);
        assert!(matches!(quoted, Ok(Incoming::Response { id: 4, .. })));
    }
}
