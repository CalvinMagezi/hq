//! The owner's view of sandbox grants (`hq_db::sandbox_grants`): what helpers asked for and how it
//! was decided, revoking one, the access profiles that are configured, and binding a session or
//! task to one. Behind the web credential, which is the owner's; no `/mcp` key of any scope and no
//! helper token reaches these routes. A helper has `sandbox_request` and nothing else.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::sandbox_grants as store;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

const LIST_LIMIT: usize = 200;
const NO_PROFILE: &str = "none";

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    session: Option<String>,
}

pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let grants = match q.session.as_deref() {
        Some(s) => store::for_session(&state.db, s, LIST_LIMIT),
        None => store::recent(&state.db, LIST_LIMIT),
    };
    match grants {
        Ok(grants) => Json(json!({ "grants": grants })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn revoke_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    match store::revoke(&state.db, &id, now) {
        Ok(true) => Json(json!({ "revoked": id })).into_response(),
        Ok(false) => ApiError::NotFound(format!("no live grant {id}")).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn profiles_handler() -> Response {
    let cfg = match hq_core::config::HqConfig::load() {
        Ok(c) => c,
        Err(e) => return ApiError::Internal(e.to_string()).into_response(),
    };
    let home = hq_core::grants::home();
    let profiles: Vec<_> = cfg
        .agent_host
        .grants
        .profiles
        .iter()
        .map(|(name, p)| {
            let r = p.resolved(&home);
            json!({
                "name": name,
                "writable": r.writable,
                "readable": r.readable,
                "loopback_ports": r.loopback_ports,
                "egress": r.egress,
            })
        })
        .collect();
    Json(json!({ "profiles": profiles })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct BindBody {
    /// `session` or `task`.
    scope: String,
    id: String,
    /// A configured profile name, or `none`.
    name: String,
}

pub(crate) async fn bind_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<BindBody>,
) -> Response {
    let cfg = match hq_core::config::HqConfig::load() {
        Ok(c) => c,
        Err(e) => return ApiError::Internal(e.to_string()).into_response(),
    };
    bind(&state.db, &cfg.agent_host.grants, &body).map_or_else(IntoResponse::into_response, |()| {
        Json(json!({ "bound": body.name })).into_response()
    })
}

fn bind(
    db: &hq_db::Database,
    grants: &hq_core::grants::GrantsConfig,
    body: &BindBody,
) -> Result<(), ApiError> {
    if body.id.trim().is_empty() || body.id.len() > 200 {
        return Err(ApiError::bad_request("id is required"));
    }
    if body.name != NO_PROFILE && !grants.profiles.contains_key(&body.name) {
        return Err(ApiError::unprocessable(format!(
            "no profile '{}' in agent_host.grants.profiles",
            body.name
        )));
    }
    let scope = match body.scope.as_str() {
        "session" => body.id.clone(),
        "task" => format!("{}{}", store::TASK_SCOPE, body.id),
        _ => return Err(ApiError::bad_request("scope must be session or task")),
    };
    store::bind_profile(db, &scope, &body.name, chrono::Utc::now().timestamp())
        .map(|_| ())
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hq_core::grants::{GrantProfile, GrantsConfig};
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
        body: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:5678")
            .header("content-type", "application/json");
        if let Some(b) = bearer {
            req = req.header("authorization", b);
        }
        let res = app
            .clone()
            .oneshot(
                req.body(Body::from(body.unwrap_or_default().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn filed(db: &hq_db::Database, session: &str, resource: &str) -> String {
        let d = hq_core::grants::SandboxDelta::default();
        store::file(
            db,
            &store::NewGrant {
                session_id: session,
                task_id: None,
                host: "local",
                repo: "",
                capability: "fs_write",
                resource,
                delta: &d,
                state: store::ACTIVE,
                decided_by: store::BY_POLICY,
                helper_reason: "",
                decision_note: "",
                expires_at: None,
            },
            1,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn only_the_owner_reaches_the_grant_routes() {
        let (state, _vault) = state(Some("s3cret"));
        let app = crate::create_router(state.clone());
        let bind = r#"{"scope":"task","id":"t1","name":"none"}"#;
        for (method, uri, body) in [
            ("GET", "/api/sandbox-grants", None),
            ("POST", "/api/sandbox-grants/abc/revoke", None),
            ("GET", "/api/sandbox-grants/profiles", None),
            ("POST", "/api/sandbox-grants/profile", Some(bind)),
        ] {
            for bearer in [None, Some("Bearer mcp-key"), Some("Bearer s3cre")] {
                let (status, _) = call(&app, method, uri, bearer, body).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} {bearer:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_owner_lists_filters_and_revokes() {
        let (state, _vault) = state(Some("s3cret"));
        let a = filed(&state.db, "s1", "/r/a");
        filed(&state.db, "s2", "/r/b");
        let app = crate::create_router(state.clone());
        let owner = Some("Bearer s3cret");
        let (status, all) = call(&app, "GET", "/api/sandbox-grants", owner, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all["grants"].as_array().unwrap().len(), 2);
        let (_, one) = call(&app, "GET", "/api/sandbox-grants?session=s1", owner, None).await;
        assert_eq!(one["grants"].as_array().unwrap().len(), 1);
        let uri = format!("/api/sandbox-grants/{a}/revoke");
        assert_eq!(
            call(&app, "POST", &uri, owner, None).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "POST", &uri, owner, None).await.0,
            StatusCode::NOT_FOUND
        );
        assert!(
            store::delta_for_session(&state.db, "s1", 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn binding_needs_a_known_profile_a_known_scope_and_an_id() {
        let db = hq_db::Database::open_memory().unwrap();
        let mut grants = GrantsConfig::default();
        grants
            .profiles
            .insert("mobile".into(), GrantProfile::default());
        let body = |scope: &str, id: &str, name: &str| BindBody {
            scope: scope.into(),
            id: id.into(),
            name: name.into(),
        };
        assert!(bind(&db, &grants, &body("task", "t1", "mobile")).is_ok());
        assert_eq!(
            store::profile_for(&db, "s1", Some("t1"))
                .unwrap()
                .as_deref(),
            Some("mobile")
        );
        assert!(bind(&db, &grants, &body("session", "s9", "none")).is_ok());
        for bad in [
            body("task", "t1", "nope"),
            body("team", "t1", "mobile"),
            body("task", " ", "mobile"),
        ] {
            assert!(bind(&db, &grants, &bad).is_err());
        }
    }
}
