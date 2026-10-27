//! The owner's list of folders they approved for connector and handoff-key sessions
//! (`hq_db::folder_grants`), and revoking one. Behind the web credential, which is the owner's;
//! no `/mcp` key of any scope reaches these routes.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

pub(crate) async fn list_handler(State(state): State<Arc<WsState>>) -> Response {
    match hq_db::folder_grants::list(&state.db) {
        Ok(grants) => Json(json!({ "grants": grants })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn revoke_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<i64>,
) -> Response {
    match hq_db::folder_grants::revoke(&state.db, id) {
        Ok(true) => Json(json!({ "revoked": id })).into_response(),
        Ok(false) => ApiError::NotFound(format!("no live folder grant {id}")).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn state(token: Option<&str>) -> (Arc<WsState>, tempfile::TempDir) {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.web_auth_token = token.map(String::from);
        (Arc::new(state), vault)
    }

    async fn call(
        app: &axum::Router,
        method: &str,
        uri: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:5678");
        if let Some(b) = bearer {
            req = req.header("authorization", b);
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn only_the_owner_lists_and_revokes_folder_grants() {
        let (state, _vault) = state(Some("s3cret"));
        hq_db::folder_grants::grant(&state.db, "laptop", "/srv/app", "claude").unwrap();
        let app = crate::create_router(state.clone());
        for (method, uri) in [
            ("GET", "/api/session-folders"),
            ("DELETE", "/api/session-folders/1"),
        ] {
            for bearer in [None, Some("Bearer mcp-key"), Some("Bearer s3cre")] {
                let (status, _) = call(&app, method, uri, bearer).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} with {bearer:?}"
                );
            }
        }
        let owner = Some("Bearer s3cret");
        let (status, body) = call(&app, "GET", "/api/session-folders", owner).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["grants"][0]["path"], "/srv/app");
        let id = body["grants"][0]["id"].as_i64().unwrap();
        let uri = format!("/api/session-folders/{id}");
        assert_eq!(call(&app, "DELETE", &uri, owner).await.0, StatusCode::OK);
        assert_eq!(
            call(&app, "DELETE", &uri, owner).await.0,
            StatusCode::NOT_FOUND
        );
        assert!(
            hq_db::folder_grants::folders_for_host(&state.db, "laptop")
                .unwrap()
                .is_empty()
        );
    }
}
