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

/// Each row, with the starter set a waiting request falls inside so the first card can offer it.
/// The suggestion reads the session computer's home, so it is worked out off the async runtime.
fn with_suggestions(grants: Vec<store::Grant>) -> Vec<serde_json::Value> {
    grants
        .into_iter()
        .map(|g| {
            let starter = hq_tools::harness_session::sandbox_access::suggested_starter(&g);
            let mut v = serde_json::to_value(&g).unwrap_or_default();
            v["suggested_starter"] = json!(starter);
            v
        })
        .collect()
}

pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let db = state.db.clone();
    let session = q.session;
    let work = tokio::task::spawn_blocking(move || {
        let grants = match session.as_deref() {
            Some(s) => store::for_session(&db, s, LIST_LIMIT),
            None => store::recent(&db, LIST_LIMIT),
        }?;
        Ok::<_, anyhow::Error>(with_suggestions(grants))
    })
    .await;
    match work {
        Ok(Ok(grants)) => Json(json!({ "grants": grants })).into_response(),
        Ok(Err(e)) => ApiError::from(e).into_response(),
        Err(e) => ApiError::Internal(e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SummaryQuery {
    days: Option<i64>,
}

const SUMMARY_DEFAULT_DAYS: i64 = 7;
const SUMMARY_MAX_DAYS: i64 = 90;
const SECS_PER_DAY: i64 = 86_400;

/// What HQ did for the owner lately, as counts. Device-only: nothing here leaves the HQ.
pub(crate) async fn summary_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<SummaryQuery>,
) -> Response {
    let days = q
        .days
        .unwrap_or(SUMMARY_DEFAULT_DAYS)
        .clamp(1, SUMMARY_MAX_DAYS);
    let now = chrono::Utc::now().timestamp();
    match store::summary(&state.db, now - days * SECS_PER_DAY, now) {
        Ok(s) => Json(json!({ "days": days, "summary": s })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// The owner's standing rules, task and repository, with what each allows.
pub(crate) async fn rules_handler(State(state): State<Arc<WsState>>) -> Response {
    let now = chrono::Utc::now().timestamp();
    match store::all_rules(&state.db, now) {
        Ok(rules) => Json(json!({ "rules": rules })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn starters_handler() -> Response {
    let starters: Vec<_> = hq_core::grants::STARTER_PROFILES
        .iter()
        .filter_map(|name| {
            let p = hq_core::grants::starter_profile(name)?;
            Some(json!({
                "name": name,
                "writable": p.writable,
                "readable": p.readable,
                "loopback_ports": p.loopback_ports,
                "egress": p.egress,
            }))
        })
        .collect();
    Json(json!({ "starters": starters })).into_response()
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
            .header("content-type", "application/json")
            .header("x-hq-client", "test");
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
            ("GET", "/api/sandbox-grants/summary", None),
            ("GET", "/api/sandbox-grants/rules", None),
            ("GET", "/api/sandbox-grants/starters", None),
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

    #[tokio::test]
    async fn the_owner_reads_starters_rules_and_the_summary() {
        let (state, _vault) = state(Some("s3cret"));
        filed(&state.db, "s1", "/r/a");
        store::add_rule(&state.db, "repo:/r", "fs_write", "~/.gradle", "x", 1, None).unwrap();
        let app = crate::create_router(state.clone());
        let owner = Some("Bearer s3cret");
        let (_, starters) = call(&app, "GET", "/api/sandbox-grants/starters", owner, None).await;
        let names: Vec<_> = starters["starters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["mobile-dev", "node-build", "rust-build"]);
        let (_, rules) = call(&app, "GET", "/api/sandbox-grants/rules", owner, None).await;
        assert_eq!(rules["rules"][0]["scope"], "repo");
        assert_eq!(rules["rules"][0]["resource"], "~/.gradle");
        let (_, sum) = call(
            &app,
            "GET",
            "/api/sandbox-grants/summary?days=7",
            owner,
            None,
        )
        .await;
        assert_eq!(sum["days"], 7);
        assert_eq!(sum["summary"]["standing_rules"], 1);
    }

    #[tokio::test]
    async fn answering_a_helper_request_with_allow_repo_leaves_a_standing_rule() {
        let (state, _vault) = state(Some("s3cret"));
        let d = hq_core::grants::SandboxDelta::default();
        let id = store::file(
            &state.db,
            &store::NewGrant {
                session_id: "s1",
                task_id: Some("t1"),
                host: "local",
                repo: "/work/app",
                capability: "fs_write",
                resource: "/work/app/cache",
                delta: &d,
                state: store::PENDING,
                decided_by: store::BY_SYSTEM,
                helper_reason: "",
                decision_note: "",
                expires_at: None,
            },
            1,
        )
        .unwrap();
        let item = hq_core::types::ValueItem::new(
            "sandbox-grants",
            hq_core::types::ValueKind::ActionNeeded,
            "t",
            "b",
        )
        .with_dedup_key(format!("sandbox-grant:{id}"));
        let item_id = item.id.clone();
        hq_db::value_items::insert(&state.db, &item).unwrap();
        let app = crate::create_router(state.clone());
        let owner = Some("Bearer s3cret");
        let uri = format!("/api/notifications/val_{item_id}/action");
        let (bad, _) = call(
            &app,
            "POST",
            &uri,
            owner,
            Some(r#"{"action":"allow_starter","starter":"nope"}"#),
        )
        .await;
        assert_eq!(
            bad,
            StatusCode::BAD_REQUEST,
            "an unknown starter is refused before anything is recorded"
        );
        assert!(
            store::rules_for(&state.db, &["repo:/work/app".into()], 10)
                .unwrap()
                .is_empty()
        );
        let (ok, _) = call(
            &app,
            "POST",
            &uri,
            owner,
            Some(r#"{"action":"allow_repo"}"#),
        )
        .await;
        assert_eq!(ok, StatusCode::OK);
        let rules = store::rules_for(&state.db, &["repo:/work/app".into()], 10).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].resource, "/work/app/cache");
        // A near-miss token cannot answer.
        let (denied, _) = call(
            &app,
            "POST",
            &uri,
            Some("Bearer s3cre"),
            Some(r#"{"action":"allow_repo"}"#),
        )
        .await;
        assert_eq!(denied, StatusCode::UNAUTHORIZED);
    }
}
