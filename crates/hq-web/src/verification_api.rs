//! Task verification REST API: the owner's side of `docs/plans/task-verification.md`.
//!
//! Everything here is behind the web credential, and the web credential is the owner's. The MCP
//! gateway (`/mcp`, any key of any scope) reaches the task tools only, never these routes, so no
//! agent can start, apply or undo a sweep, answer an audit, veto a task or ask for a second
//! opinion. The heavy work (reading GitHub, asking a reviewer, moving tasks) is done by the
//! daemon a few items per tick; these handlers record what the owner asked for and read what
//! is there.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::tasks as t;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

fn respond(result: Result<Value, anyhow::Error>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

fn tell(state: &Arc<WsState>, ids: Vec<String>) {
    crate::tasks_api::broadcast_related(state, ids);
}

// ---- status, queue, briefing ----

/// Mode, calibration, queue health and what waits on the owner, for the tasks page's bar.
pub(crate) async fn status_handler(State(state): State<Arc<WsState>>) -> Response {
    respond(state.db.with_conn(|c| {
        let cfg = t::verification_policy(c)?;
        Ok(json!({
            "mode": cfg.mode,
            "autonomy_gate": cfg.autonomy_gate,
            "confirm_over": cfg.sweep_confirm_over,
            "calibration": t::calibration(c, &cfg)?,
            "queue": t::queue_health(c, &cfg)?,
            "rollout": t::rollout(c, &cfg)?,
            "holds": t::list_holds(c)?.len(),
            "active_sweeps": t::active_sweeps(c)?.len(),
        }))
    }))
}

pub(crate) async fn queue_handler(State(state): State<Arc<WsState>>) -> Response {
    respond(state.db.with_conn(|c| {
        let cfg = t::verification_policy(c)?;
        Ok(serde_json::to_value(t::queue_health(c, &cfg)?)?)
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct BriefingParams {
    /// Hours back from now. Default 24.
    hours: Option<i64>,
    /// Or an absolute start, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` (UTC).
    since: Option<String>,
}

pub(crate) async fn briefing_handler(
    State(state): State<Arc<WsState>>,
    Query(p): Query<BriefingParams>,
) -> Response {
    let since = match p.since.filter(|s| !s.trim().is_empty()) {
        Some(s) => match t::normalize_since(&s) {
            Ok(s) => s,
            Err(e) => return ApiError::bad_request(e.to_string()).into_response(),
        },
        None => (chrono::Utc::now()
            - chrono::Duration::hours(p.hours.unwrap_or(24).clamp(1, 24 * 31)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string(),
    };
    respond(state.db.with_conn(move |c| {
        let cfg = t::verification_policy(c)?;
        Ok(serde_json::to_value(t::briefing(c, &cfg, &since)?)?)
    }))
}

// ---- sweeps ----

#[derive(Debug, Deserialize)]
pub(crate) struct NewSweepBody {
    scope: t::SweepScope,
}

pub(crate) async fn create_sweep_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<NewSweepBody>,
) -> Response {
    respond(state.db.with_conn(move |c| {
        let cfg = t::verification_policy(c)?;
        let sweep = hq_tools::tasks::verify::start_sweep(c, &cfg, &body.scope)?;
        Ok(serde_json::to_value(sweep)?)
    }))
}

pub(crate) async fn list_sweeps_handler(State(state): State<Arc<WsState>>) -> Response {
    respond(
        state
            .db
            .with_conn(|c| Ok(json!({ "sweeps": t::list_sweeps(c, 20)? }))),
    )
}

pub(crate) async fn get_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    respond(state.db.with_conn(move |c| {
        let sweep = t::get_sweep(c, &id)?
            .ok_or_else(|| anyhow::Error::from(ApiError::NotFound("no such sweep".into())))?;
        let cfg = t::verification_policy(c)?;
        let preview = (sweep.status == t::SWEEP_READY)
            .then(|| t::apply_preview(c, &cfg, &id))
            .transpose()?;
        Ok(json!({
            "sweep": sweep,
            "progress": t::sweep_summary(c, &id)?,
            "items": t::sweep_items(c, &id)?,
            "apply_preview": preview,
        }))
    }))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SweepActionBody {
    /// Confirms an apply that moves more than `sweep_confirm_over` tasks.
    #[serde(default)]
    confirm: bool,
}

async fn sweep_action(
    state: Arc<WsState>,
    id: String,
    action: &'static str,
    confirm: bool,
) -> Response {
    respond(state.db.with_conn(move |c| {
        let cfg = t::verification_policy(c)?;
        let sweep = hq_tools::tasks::verify::sweep_action(c, &cfg, &id, action, confirm)?;
        Ok(serde_json::to_value(sweep)?)
    }))
}

pub(crate) async fn run_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    sweep_action(state, id, "run", false).await
}

pub(crate) async fn pause_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    sweep_action(state, id, "pause", false).await
}

pub(crate) async fn cancel_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    sweep_action(state, id, "cancel", false).await
}

pub(crate) async fn apply_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<SweepActionBody>>,
) -> Response {
    let confirm = body.map(|b| b.0.confirm).unwrap_or(false);
    sweep_action(state, id, "apply", confirm).await
}

pub(crate) async fn undo_sweep_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let result = state
        .db
        .with_conn(move |c| hq_tools::tasks::verify::undo_sweep(c, &id));
    match result {
        Ok(report) => {
            // The task watcher tells open boards that the tasks changed.
            Json(json!({
                "restored": report.restored,
                "left_alone": report.left_alone.iter()
                    .map(|(task, why)| json!({ "task": task, "why": why }))
                    .collect::<Vec<_>>(),
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

// ---- audits ----

#[derive(Debug, Deserialize)]
pub(crate) struct AuditParams {
    /// Draw this many more audits now (stratified toward what has the least data).
    n: Option<usize>,
}

fn audit_card(c: &rusqlite::Connection, a: &t::Audit) -> Result<Value, anyhow::Error> {
    let task = t::get_task(c, &a.task_id)?;
    let verdict = t::get_verdict(c, &a.verdict_id)?;
    let links: Vec<Value> = t::list_task_links(c, &a.task_id)?
        .into_iter()
        .filter(|l| matches!(l.kind.as_str(), "pr" | "commit" | "url" | "vault_note"))
        .map(|l| json!({ "kind": l.kind, "ref": l.reference }))
        .collect();
    let observations: Vec<Value> = verdict
        .as_ref()
        .and_then(|v| v.observations.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|o| json!({ "id": o["id"], "kind": o["kind"], "fact": o["fact"], "ok": o["ok"] }))
        .collect();
    Ok(json!({
        "audit": a,
        "task": task.as_ref().map(|t| json!({
            "id": t.id, "display_id": t.display_id, "title": t.title, "status": t.status,
        })),
        "verdict": verdict.as_ref().map(|v| json!({
            "verdict": v.verdict, "level": v.level, "tier": v.tier, "artifact": v.artifact,
            "verifier": v.verifier, "independence": v.independence, "reasons": v.reasons,
            "criteria": v.criteria, "created_at": v.created_at,
        })),
        "observations": observations,
        "links": links,
    }))
}

/// The audit stack: `?n=10` draws ten more first. Also the running error estimate and its sample.
pub(crate) async fn audits_handler(
    State(state): State<Arc<WsState>>,
    Query(p): Query<AuditParams>,
) -> Response {
    respond(state.db.with_conn(move |c| {
        let cfg = t::verification_policy(c)?;
        let drawn = match p.n {
            Some(n) if n > 0 => {
                let seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(1) as u64;
                t::select_audits(c, n.min(100), t::AUDIT_PULL, seed)?.len()
            }
            _ => 0,
        };
        let open = t::open_audits(c, 100)?;
        let cards = open
            .iter()
            .map(|a| audit_card(c, a))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "drawn": drawn,
            "cards": cards,
            "calibration": t::calibration(c, &cfg)?,
        }))
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnswerBody {
    /// `agree` or `disagree`.
    answer: String,
    #[serde(default)]
    note: String,
}

pub(crate) async fn answer_audit_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<AnswerBody>,
) -> Response {
    let agrees = match body.answer.as_str() {
        "agree" => true,
        "disagree" => false,
        _ => return ApiError::bad_request("answer is agree or disagree").into_response(),
    };
    let result = state
        .db
        .with_conn(move |c| t::answer_audit(c, &id, agrees, &body.note, t::WEB_ACTOR));
    match result {
        Ok(audit) => {
            tell(&state, vec![audit.task_id.clone()]);
            Json(json!({ "audit": audit })).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

// ---- veto windows, second opinions, one task ----

pub(crate) async fn holds_handler(State(state): State<Arc<WsState>>) -> Response {
    respond(state.db.with_conn(|c| {
        let mut out = Vec::new();
        for h in t::list_holds(c)? {
            if let Some(task) = t::get_task(c, &h.task_id)? {
                out.push(json!({
                    "task_id": task.id, "display_id": task.display_id, "title": task.title,
                    "release_at": h.release_at,
                }));
            }
        }
        Ok(json!({ "holds": out }))
    }))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct NoteBody {
    #[serde(default)]
    note: String,
}

pub(crate) async fn veto_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<NoteBody>>,
) -> Response {
    let note = body.map(|b| b.0.note).unwrap_or_default();
    let result = state.db.with_conn(move |c| {
        t::veto_hold(c, &id, &note, t::WEB_ACTOR)?;
        Ok(t::get_task(c, &id)?.map(|task| task.id))
    });
    match result {
        Ok(task_id) => {
            tell(&state, task_id.into_iter().collect());
            Json(json!({ "vetoed": true })).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn reverify_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    respond(
        state
            .db
            .with_conn(move |c| Ok(serde_json::to_value(t::request_reverify(c, &id)?)?)),
    )
}

/// One task's verification: the verdicts it has had, who it waits on, its veto window and any
/// second opinion. For the task drawer.
pub(crate) async fn task_verification_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    respond(state.db.with_conn(move |c| {
        let task = t::get_task(c, &id)?
            .ok_or_else(|| anyhow::Error::from(ApiError::NotFound("no task found for that id".into())))?;
        let verdicts: Vec<Value> = t::list_verdicts(c, &task.id, 10)?
            .into_iter()
            .map(|v| {
                json!({
                    "id": v.id, "verdict": v.verdict, "level": v.level, "tier": v.tier,
                    "artifact": v.artifact, "verifier": v.verifier, "applied": v.applied,
                    "independence": v.independence, "reasons": v.reasons, "criteria": v.criteria,
                    "waiting_for": v.waiting_for, "created_at": v.created_at,
                    "observations": v.observations.as_array().cloned().unwrap_or_default().into_iter()
                        .map(|o| json!({ "id": o["id"], "kind": o["kind"], "fact": o["fact"], "ok": o["ok"] }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let waiting = t::waiting_state(c, &task)?;
        let hold = t::get_hold(c, &task.id)?;
        Ok(json!({
            "verdicts": verdicts,
            "waiting_for": waiting.map(|w| json!({ "who": w.who, "kind": w.kind, "reason": w.reason })),
            "hold": hold.map(|h| json!({ "release_at": h.release_at })),
            "second_opinion": t::get_reverify(c, &task.id)?,
        }))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn state(token: Option<&str>) -> (Arc<WsState>, tempfile::TempDir) {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.web_auth_token = token.map(String::from);
        let state = Arc::new(state);
        state
            .db
            .with_conn(|c| {
                t::create_initiative(c, "in-a", "personal", None, "Work", "work", "WRK")?;
                t::create_task(
                    c,
                    "tk-a",
                    "in-a",
                    &t::NewTask {
                        title: "Add the guard",
                        created_by: "test",
                        ..Default::default()
                    },
                )?;
                Ok(())
            })
            .unwrap();
        (state, vault)
    }

    fn mode(state: &WsState, mode: hq_core::config::VerificationMode) {
        let cfg = hq_core::config::VerificationConfig {
            mode,
            ..Default::default()
        };
        state
            .db
            .with_conn(|c| t::sync_verification_policy(c, &cfg))
            .unwrap();
    }

    fn submit(state: &WsState) {
        for status in [t::STATUS_IN_PROGRESS, t::STATUS_READY_FOR_REVIEW] {
            state
                .db
                .with_conn(|c| {
                    t::update_task_as(
                        c,
                        "tk-a",
                        &t::TaskPatch {
                            status: Some(status.into()),
                            ..Default::default()
                        },
                        None,
                        &t::WriteCtx {
                            actor: Some("worker"),
                            ..Default::default()
                        },
                    )
                })
                .unwrap();
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        auth: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:5678");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        let req = match body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    const ROUTES: &[(&str, &str)] = &[
        ("GET", "/api/verification/status"),
        ("GET", "/api/verification/queue"),
        ("GET", "/api/verification/briefing"),
        ("GET", "/api/verification/sweeps"),
        ("POST", "/api/verification/sweeps"),
        ("GET", "/api/verification/sweeps/sw-1"),
        ("POST", "/api/verification/sweeps/sw-1/run"),
        ("POST", "/api/verification/sweeps/sw-1/pause"),
        ("POST", "/api/verification/sweeps/sw-1/cancel"),
        ("POST", "/api/verification/sweeps/sw-1/apply"),
        ("POST", "/api/verification/sweeps/sw-1/undo"),
        ("GET", "/api/verification/audits"),
        ("POST", "/api/verification/audits/au-1"),
        ("GET", "/api/verification/holds"),
        ("POST", "/api/verification/holds/tk-a/veto"),
        ("GET", "/api/tasks/tk-a/verification"),
        ("POST", "/api/tasks/tk-a/reverify"),
    ];

    #[tokio::test]
    async fn no_credential_but_the_web_one_reaches_any_verification_route() {
        let (state, _vault) = state(Some("s3cret"));
        let app = crate::create_router(state);
        for (method, uri) in ROUTES {
            // No credential, an MCP-style key in either header, and a wrong bearer: all refused.
            for headers in [None, Some("Bearer mcp-key"), Some("Bearer s3cre")] {
                let (status, _) = call(&app, method, uri, headers, Some(json!({}))).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} with {headers:?}"
                );
            }
            let req = Request::builder()
                .method(*method)
                .uri(*uri)
                .header("host", "127.0.0.1:5678")
                .header("x-api-key", "mcp-key")
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                app.clone().oneshot(req).await.unwrap().status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri} with an x-api-key"
            );
            let (status, _) = call(&app, method, uri, Some("Bearer s3cret"), Some(json!({}))).await;
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {uri} for the owner"
            );
        }
    }

    #[tokio::test]
    async fn lite_serves_none_of_it() {
        let vault = tempfile::tempdir().unwrap();
        let st = WsState::new(vault.path().to_path_buf(), None)
            .with_profile(hq_core::config::Profile::Lite);
        let app = crate::create_router(Arc::new(st));
        for (method, uri) in ROUTES {
            let (status, _) = call(&app, method, uri, None, Some(json!({}))).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn status_and_the_queue_read_on_a_fresh_install_and_a_sweep_waits_for_verification_to_be_on()
     {
        let (state, _vault) = state(None);
        let app = crate::create_router(state.clone());
        let (status, body) = call(&app, "GET", "/api/verification/status", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["mode"], "off");
        assert_eq!(body["calibration"]["tiers"][1]["state"], "insufficient");
        assert_eq!(body["active_sweeps"], 0);
        assert_eq!(body["rollout"]["stage"], "off");
        assert!(
            body["rollout"]["next"]["change"]
                .as_str()
                .unwrap()
                .contains("mode: shadow"),
            "{body}"
        );

        submit(&state);
        let (status, body) = call(
            &app,
            "POST",
            "/api/verification/sweeps",
            None,
            Some(json!({"scope": {"kind": "all"}})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("verification is off"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_sweep_is_created_read_and_controlled_over_http_and_cannot_skip_a_step() {
        let (state, _vault) = state(None);
        mode(&state, hq_core::config::VerificationMode::Enforce);
        submit(&state);
        let app = crate::create_router(state.clone());

        let (status, body) = call(
            &app,
            "POST",
            "/api/verification/sweeps",
            None,
            Some(json!({"scope": {"kind": "initiative", "id": "in-a"}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "planning");
        let id = body["id"].as_str().unwrap().to_string();

        let (_, one) = call(
            &app,
            "GET",
            &format!("/api/verification/sweeps/{id}"),
            None,
            None,
        )
        .await;
        assert_eq!(one["items"].as_array().unwrap().len(), 1);
        assert_eq!(one["items"][0]["display_id"], "WRK-001");
        let (_, list) = call(&app, "GET", "/api/verification/sweeps", None, None).await;
        assert_eq!(list["sweeps"].as_array().unwrap().len(), 1);

        // Still planning: it cannot be run or applied, and the answer says why.
        for action in ["run", "apply"] {
            let (status, body) = call(
                &app,
                "POST",
                &format!("/api/verification/sweeps/{id}/{action}"),
                None,
                Some(json!({"confirm": true})),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{action}");
            assert!(
                body["error"].as_str().unwrap().contains("planning"),
                "{body}"
            );
        }
        let (status, body) = call(
            &app,
            "POST",
            &format!("/api/verification/sweeps/{id}/cancel"),
            None,
            None,
        )
        .await;
        assert_eq!(
            (status, body["status"].as_str()),
            (StatusCode::OK, Some("cancelled"))
        );
        // Undoing a sweep that was never applied is refused.
        let (status, _) = call(
            &app,
            "POST",
            &format!("/api/verification/sweeps/{id}/undo"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Nothing was waiting in an initiative that does not exist.
        let (status, _) = call(
            &app,
            "POST",
            "/api/verification/sweeps",
            None,
            Some(json!({"scope": {"kind": "initiative", "id": "in-none"}})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_briefing_the_audit_stack_and_a_task_s_verification_read_and_validate() {
        let (state, _vault) = state(None);
        mode(&state, hq_core::config::VerificationMode::Enforce);
        submit(&state);
        let app = crate::create_router(state.clone());

        let (status, b) = call(
            &app,
            "GET",
            "/api/verification/briefing?hours=72",
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(b["mode"], "enforce");
        assert_eq!(b["completed_total"], 0);
        assert!(b["health"]["backlog"].is_object());
        let (status, _) = call(
            &app,
            "GET",
            "/api/verification/briefing?since=not-a-date",
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, a) = call(&app, "GET", "/api/verification/audits?n=5", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(a["drawn"], 0, "nothing completed yet to audit");
        assert!(a["cards"].as_array().unwrap().is_empty());
        let (status, _) = call(
            &app,
            "POST",
            "/api/verification/audits/au-none",
            None,
            Some(json!({"answer": "maybe"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = call(
            &app,
            "POST",
            "/api/verification/audits/au-none",
            None,
            Some(json!({"answer": "agree"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"].as_str().unwrap().contains("no audit"),
            "{body}"
        );

        let (status, v) = call(&app, "GET", "/api/tasks/tk-a/verification", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(v["verdicts"].as_array().unwrap().is_empty());
        assert_eq!(v["waiting_for"]["who"], "verifier");
        let (status, _) = call(&app, "GET", "/api/tasks/nope/verification", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // A second opinion needs a verdict to disagree with.
        let (status, body) = call(&app, "POST", "/api/tasks/tk-a/reverify", None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"].as_str().unwrap().contains("no verdict yet"),
            "{body}"
        );
        let (status, _) = call(
            &app,
            "POST",
            "/api/verification/holds/tk-a/veto",
            None,
            Some(json!({"note": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "no veto window is open");
    }
}
