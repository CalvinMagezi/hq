//! Helper setup for the owner's apps: which helpers each connected computer has, and buttons to
//! set them up, sign them in or look again. Behind the owner's web token: the owner pressing the
//! button is the approval, so nothing here waits in the inbox. The commands themselves are fixed
//! in `hq_tools::helper_setup::catalog`; a request names a computer, an action and helper ids and
//! nothing else (`docs/HELPER_SETUP.md`).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use hq_tools::helper_setup::{self, catalog, service};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;

#[derive(Deserialize, Default)]
pub(crate) struct ListQuery {
    /// Look at the computers now instead of reporting what was last seen.
    #[serde(default)]
    refresh: bool,
}

/// `GET /api/helper-setup`
pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    if q.refresh {
        let db = state.db.clone();
        let refreshed = tokio::task::spawn_blocking(move || {
            for host in hq_tools::agent_host::all_hosts()? {
                // A computer that does not answer keeps what was last seen.
                if host.version().is_ok() {
                    helper_setup::refresh_installed(host.as_ref(), &db)?;
                }
            }
            anyhow::Ok(())
        })
        .await;
        if let Ok(Err(e)) | Err(e) = refreshed.map_err(anyhow::Error::from) {
            tracing::warn!(error = %e, "refreshing helper status failed");
        }
    }
    match view(&state.db) {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

fn view(db: &hq_db::Database) -> anyhow::Result<Value> {
    let known = service::known(db)?;
    let runs = hq_db::helper_status::latest_runs(db)?;
    let hosts: Vec<Value> = hq_tools::agent_host::native_host_names()
        .into_iter()
        .map(|name| {
            let helpers: Vec<Value> = catalog::CATALOG
                .iter()
                .map(|h| {
                    let row = known
                        .get(&name)
                        .and_then(|rows| rows.iter().find(|r| r.helper == h.id));
                    let run = runs.iter().find(|r| r.host == name && r.helper == h.id);
                    json!({
                        "id": h.id, "title": h.title, "about": h.blurb, "free": h.free_note,
                        "installed": row.map(|r| r.installed),
                        "signed_in": row.and_then(|r| r.signed_in),
                        "checked_at": row.map(|r| r.checked_at),
                        "error": row.and_then(|r| r.last_error.clone()),
                        "run": run,
                    })
                })
                .collect();
            json!({ "host": name, "helpers": helpers })
        })
        .collect();
    Ok(json!({ "hosts": hosts }))
}

#[derive(Deserialize, Default)]
pub(crate) struct ActionBody {
    #[serde(default)]
    helpers: Vec<String>,
}

/// `POST /api/helper-setup/{host}/{action}` with `{ "helpers": [...] }` (empty: all of them).
/// `setup`, `install` and `login` start in the background and answer at once; the list shows how
/// they go. `check` looks now and answers when done.
pub(crate) async fn action_handler(
    State(state): State<Arc<WsState>>,
    Path((host, action)): Path<(String, String)>,
    body: Option<Json<ActionBody>>,
) -> Response {
    let ids = body.map(|b| b.0.helpers).unwrap_or_default();
    if let Err(e) = service::pick(&ids) {
        return ApiError::bad_request(e.to_string()).into_response();
    }
    let name = host.clone();
    let built = tokio::task::spawn_blocking(move || hq_tools::agent_host::host(Some(&name))).await;
    let host = match built {
        Ok(Ok(host)) => host,
        Ok(Err(e)) => return ApiError::bad_request(e.to_string()).into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if action == "check" {
        let db = state.db.clone();
        let result = tokio::task::spawn_blocking(move || {
            service::pick(&ids)?
                .into_iter()
                .map(|h| service::check(host.as_ref(), &db, h))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await;
        return match result {
            Ok(Ok(reports)) => Json(json!({ "helpers": reports })).into_response(),
            Ok(Err(e)) => ApiError::bad_request(e.to_string()).into_response(),
            Err(e) => ApiError::internal(e).into_response(),
        };
    }
    match helper_setup::start(state.db.clone(), host, &action, &ids) {
        Ok(titles) => Json(json!({ "started": true, "helpers": titles })).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}
