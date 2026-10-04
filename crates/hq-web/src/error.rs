//! The one error type every hq-web handler returns, so a status never depends on
//! message wording and every error body has the shape `{"error": "..."}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// hq-db reports a lost claim race as an untyped error with this marker. The
/// task board refreshes only on 409, so it has to map to a conflict.
const CLAIM_CONFLICT_MARKER: &str = "(claim conflict)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApiError {
    BadRequest(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    Unavailable(String),
    Internal(String),
}

impl ApiError {
    pub(crate) fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub(crate) fn not_found() -> Self {
        Self::NotFound("not found".to_string())
    }

    pub(crate) fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::BadRequest(m)
            | Self::Forbidden(m)
            | Self::NotFound(m)
            | Self::Conflict(m)
            | Self::Unavailable(m)
            | Self::Internal(m) => m,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status(), Json(json!({ "error": self.message() }))).into_response()
    }
}

/// Errors from hq-db and the shared task helpers arrive as `anyhow`. A typed
/// `ApiError` inside keeps its status; otherwise a database fault is a 500 and
/// anything else is the caller's input.
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(api) = e.downcast_ref::<ApiError>() {
            return api.clone();
        }
        let msg = e.to_string();
        if msg.contains(CLAIM_CONFLICT_MARKER) {
            return Self::Conflict(msg);
        }
        if e.chain().any(|c| c.is::<rusqlite::Error>()) {
            return Self::Internal(msg);
        }
        Self::BadRequest(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_errors_keep_their_status_through_anyhow() {
        let wrapped: anyhow::Error = ApiError::NotFound("task x".into()).into();
        assert_eq!(ApiError::from(wrapped).status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn untyped_errors_are_classified() {
        let claim = anyhow::anyhow!("task moved on (claim conflict)");
        assert_eq!(ApiError::from(claim).status(), StatusCode::CONFLICT);
        let db: anyhow::Error = rusqlite::Error::QueryReturnedNoRows.into();
        assert_eq!(ApiError::from(db).status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(ApiError::from(anyhow::anyhow!("bad slug")).status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn body_is_a_single_error_field() {
        let res = ApiError::bad_request("nope").into_response();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"error": "nope"}));
    }
}
