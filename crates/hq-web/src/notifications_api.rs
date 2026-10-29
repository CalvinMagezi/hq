//! Notifications API over the value bus's `value_items` table: list, approve,
//! dismiss and mark all read, each broadcast to open tabs.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::WsState;
use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationKind {
    /// Waits on the user's approve or reject.
    ActionNeeded,
    ValueItem,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationState {
    Pending,
    Approved,
    Dismissed,
    Expired,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct NotificationMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) score: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    /// Ordered, so the same list always serializes to the same bytes (its ETag depends on it).
    pub(crate) extra: BTreeMap<String, String>,
    /// A page of the web app the item opens, from `notify_owner`'s `link`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NotificationItem {
    pub(crate) id: String,
    pub(crate) kind: NotificationKind,
    pub(crate) category: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) state: NotificationState,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) metadata: NotificationMetadata,
    pub(crate) actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NotificationsResponse {
    pub(crate) notifications: Vec<NotificationItem>,
    pub(crate) unread_count: usize,
    pub(crate) pending_approvals_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListNotificationsParams {
    pub(crate) state: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NotificationActionRequest {
    pub(crate) action: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct NotificationActionResponse {
    pub(crate) success: bool,
    pub(crate) new_state: String,
    pub(crate) message: String,
}

const VALUE_ITEM_COLUMNS: &str = "SELECT id, source_task, kind, title, body, artifact_path, score, state, created_at, link FROM value_items";

fn state_where(state_filter: Option<&str>) -> &'static str {
    match state_filter {
        Some("pending") => " WHERE state IN ('pending', 'delivered')",
        Some("approved") => " WHERE state = 'engaged'",
        Some("dismissed") => " WHERE state = 'dismissed'",
        Some("resolved") | Some("history") => " WHERE state IN ('engaged', 'dismissed', 'expired')",
        _ => "",
    }
}

fn notification_from_row(row: &rusqlite::Row) -> rusqlite::Result<NotificationItem> {
    let id: String = row.get(0)?;
    let source_task: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let state_str: String = row.get(7)?;
    let created_at_str: String = row.get(8)?;
    let state = match state_str.as_str() {
        "engaged" => NotificationState::Approved,
        "dismissed" => NotificationState::Dismissed,
        "expired" => NotificationState::Expired,
        _ => NotificationState::Pending,
    };
    let needs_answer = kind_str == "proposal" || kind_str == "action_needed";
    let kind = if needs_answer {
        NotificationKind::ActionNeeded
    } else {
        NotificationKind::ValueItem
    };
    let category = if source_task == "email_triage" {
        "system"
    } else {
        "value"
    };
    let actions: &[&str] = match (state == NotificationState::Pending, needs_answer) {
        (false, _) => &[],
        (true, true) => &["approve", "reject"],
        (true, false) => &["dismiss", "acknowledge"],
    };
    let extra = BTreeMap::from([
        ("source_task".to_string(), source_task),
        ("val_kind".to_string(), kind_str),
    ]);
    Ok(NotificationItem {
        id: format!("val_{id}"),
        kind,
        category: category.to_string(),
        title: row.get(3)?,
        description: row.get(4)?,
        state,
        created_at: created_at_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
        metadata: NotificationMetadata {
            artifact_path: row.get(5)?,
            score: Some(row.get(6)?),
            extra,
            link: row
                .get::<_, Option<String>>(9)?
                .filter(|l| crate::web_push::is_app_path(l)),
        },
        actions: actions.iter().map(|a| a.to_string()).collect(),
    })
}

/// Collect value-bus items from the SQLite database.
pub(crate) fn scan_value_items(
    db: &hq_db::Database,
    state_filter: Option<&str>,
) -> Result<Vec<NotificationItem>, anyhow::Error> {
    db.with_conn(|conn| {
        let sql = format!("{VALUE_ITEM_COLUMNS}{}", state_where(state_filter));
        let mut stmt = conn.prepare(&sql)?;
        let items = stmt
            .query_map([], notification_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(items)
    })
}

/// Aggregate all notifications matching filters.
pub(crate) fn aggregate_notifications(
    db: &hq_db::Database,
    state_filter: Option<&str>,
    category_filter: Option<&str>,
    limit: usize,
) -> Result<NotificationsResponse, anyhow::Error> {
    let mut all = scan_value_items(db, state_filter)?;
    if let Some(cat) = category_filter.filter(|c| *c != "all") {
        all.retain(|n| n.category == cat);
    }
    all.sort_by_key(|a| std::cmp::Reverse(a.created_at));

    let pending = || all.iter().filter(|n| n.state == NotificationState::Pending);
    let pending_approvals_count = pending()
        .filter(|n| n.kind == NotificationKind::ActionNeeded)
        .count();
    let unread_count = pending().count();
    all.truncate(limit);

    Ok(NotificationsResponse {
        notifications: all,
        unread_count,
        pending_approvals_count,
    })
}

/// Maps a client action to the value item's `(engagement, state)`.
fn action_outcome(action: &str) -> Option<(&'static str, &'static str)> {
    match action {
        "approve" | "engage" => Some(("approved", "engaged")),
        "dismiss" | "reject" => Some(("dismissed", "dismissed")),
        _ => None,
    }
}

/// Returns false when no value item has that id.
fn record_action(
    db: &hq_db::Database,
    val_id: &str,
    engagement: &str,
    db_state: &str,
) -> anyhow::Result<bool> {
    // The token path only matches delivered items; anything else is updated by exact id.
    if hq_db::value_items::record_engagement_by_token(db, val_id, engagement)? {
        return Ok(true);
    }
    let updated = db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE value_items SET state=?2, engagement=?3, engaged_at=datetime('now') WHERE id=?1",
            rusqlite::params![val_id, db_state, engagement],
        )?)
    })?;
    Ok(updated > 0)
}

pub(crate) async fn list_notifications_handler(
    State(state): State<Arc<WsState>>,
    Query(params): Query<ListNotificationsParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let limit = params.limit.unwrap_or(50);
    match aggregate_notifications(
        &state.db,
        params.state.as_deref(),
        params.category.as_deref(),
        limit,
    ) {
        Ok(res) => with_etag(&res, &headers),
        Err(e) => {
            warn!(error = %e, "list_notifications_handler failed");
            ApiError::internal(e).into_response()
        }
    }
}

/// The list as JSON with an ETag over its bytes. A client that sends back the ETag it holds gets
/// 304 and no body while nothing changed, so a phone that polls a server HQ over mobile data
/// pays a few hundred bytes a poll instead of the whole list.
fn with_etag(res: &NotificationsResponse, headers: &axum::http::HeaderMap) -> Response {
    use axum::http::{StatusCode, header};
    use sha2::{Digest, Sha256};
    let body = match serde_json::to_vec(res) {
        Ok(body) => body,
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let digest = Sha256::digest(&body);
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    let etag = format!("\"{hex}\"");
    let held = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    if held {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::ETAG, etag),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn notification_action_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<NotificationActionRequest>,
) -> Response {
    let Some(val_id) = id.strip_prefix("val_") else {
        return ApiError::bad_request(format!("unrecognized notification ID format '{id}'"))
            .into_response();
    };
    let Some((engagement, db_state)) = action_outcome(&body.action) else {
        return ApiError::bad_request(format!("unknown action '{}' for value item", body.action))
            .into_response();
    };
    match record_action(&state.db, val_id, engagement, db_state) {
        Err(e) => {
            warn!(id = %id, action = %body.action, error = %e, "notification action failed");
            return ApiError::internal(e).into_response();
        }
        Ok(false) => return ApiError::not_found().into_response(),
        Ok(true) => {}
    }
    info!(id = %id, action = %body.action, new_state = engagement, "notification action executed");
    state.broadcast(
        &serde_json::json!({
            "type": "notification:resolved",
            "id": id,
            "action": body.action,
            "new_state": engagement,
        })
        .to_string(),
    );
    Json(NotificationActionResponse {
        success: true,
        new_state: engagement.to_string(),
        message: format!("Value item {engagement}"),
    })
    .into_response()
}

pub(crate) async fn mark_all_read_handler(State(state): State<Arc<WsState>>) -> Response {
    let dismissed = state.db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE value_items SET state='dismissed', engagement='dismissed', engaged_at=datetime('now') WHERE state IN ('pending', 'delivered', 'routed')",
            [],
        )?)
    });
    if let Err(e) = dismissed {
        warn!(error = %e, "mark_all_read_handler failed");
        return ApiError::internal(e).into_response();
    }
    state.broadcast(
        &serde_json::json!({
            "type": "notification:badge_count",
            "unread_count": 0,
            "pending_approvals_count": 0,
        })
        .to_string(),
    );
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn state_where_ignores_unknown_and_hostile_filters() {
        for filter in [
            "",
            "bogus",
            "pending' OR 1=1 --",
            "approved; DROP TABLE value_items",
        ] {
            assert_eq!(state_where(Some(filter)), "", "{filter}");
        }
        assert_eq!(state_where(None), "");
        assert!(state_where(Some("pending")).starts_with(" WHERE state IN"));
    }

    use super::*;

    #[tokio::test]
    async fn test_aggregate_empty_notifications() {
        let db = hq_db::Database::open_memory().unwrap();
        let res = aggregate_notifications(&db, Some("pending"), None, 50).unwrap();
        assert_eq!(res.notifications.len(), 0);
        assert_eq!(res.unread_count, 0);
        assert_eq!(res.pending_approvals_count, 0);
    }

    #[tokio::test]
    async fn test_scan_value_items() {
        let db = hq_db::Database::open_memory().unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS value_items (
                    id TEXT PRIMARY KEY,
                    source_task TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL,
                    artifact_path TEXT,
                    score REAL NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL
                )",
                [],
            )?;
            conn.execute(
                "INSERT INTO value_items (id, source_task, kind, title, body, artifact_path, score, state, created_at)
                 VALUES ('val_1', 'cron:triage', 'action_needed', 'High memory usage', 'Server is near RAM limit', NULL, 0.95, 'pending', '2026-08-18T10:00:00Z')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let items = scan_value_items(&db, Some("pending")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "val_val_1");
        assert_eq!(items[0].title, "High memory usage");
        assert_eq!(items[0].state, NotificationState::Pending);
        assert_eq!(items[0].kind, NotificationKind::ActionNeeded);
        assert_eq!(items[0].actions, ["approve", "reject"]);

        let res = aggregate_notifications(&db, Some("pending"), Some("all"), 50).unwrap();
        assert_eq!(res.notifications.len(), 1);
        assert_eq!(res.pending_approvals_count, 1);
    }

    #[tokio::test]
    async fn an_unchanged_list_is_a_bodiless_304_and_a_change_is_a_new_etag() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let insert = |id: &str| {
            let id = id.to_string();
            state
                .db
                .with_conn(move |c| {
                    Ok(c.execute(
                        "INSERT INTO value_items (id, source_task, kind, title, body, state, created_at)
                         VALUES (?1, 't', 'action_needed', 'T', 'B', 'pending', '2026-09-25T00:00:00Z')",
                        [id],
                    )?)
                })
                .unwrap();
        };
        let list = |held: Option<String>| {
            let mut headers = HeaderMap::new();
            if let Some(tag) = held {
                headers.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&tag).unwrap());
            }
            let params = ListNotificationsParams {
                state: Some("pending".into()),
                category: None,
                limit: Some(30),
            };
            list_notifications_handler(State(state.clone()), Query(params), headers)
        };
        insert("first111");
        let first = list(None).await;
        assert_eq!(first.status(), 200);
        let tag = first.headers()[header::ETAG].to_str().unwrap().to_string();
        assert!(tag.starts_with('"') && tag.ends_with('"'), "{tag}");

        let same = list(Some(tag.clone())).await;
        assert_eq!(same.status(), 304);
        let body = axum::body::to_bytes(same.into_body(), 1024).await.unwrap();
        assert!(body.is_empty(), "a 304 carries no list");

        insert("second22");
        let changed = list(Some(tag.clone())).await;
        assert_eq!(changed.status(), 200, "a new item is a new list");
        assert_ne!(changed.headers()[header::ETAG].to_str().unwrap(), tag);
    }

    #[tokio::test]
    async fn action_approves_known_items_and_404s_unknown_ones() {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        state
            .db
            .with_conn(|c| {
                Ok(c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, state, created_at)
                     VALUES ('abc12345', 't', 'action_needed', 'T', 'B', 'pending', '2026-09-25T00:00:00Z')",
                    [],
                )?)
            })
            .unwrap();
        let act = |id: &str, action: &str| {
            let body = NotificationActionRequest {
                action: action.into(),
            };
            notification_action_handler(State(state.clone()), AxumPath(id.into()), Json(body))
        };
        assert_eq!(act("val_abc12345", "approve").await.status(), 200);
        assert_eq!(act("val_missing99", "approve").await.status(), 404);
        assert_eq!(act("val_abc12345", "explode").await.status(), 400);
        let approved = aggregate_notifications(&state.db, Some("approved"), None, 10).unwrap();
        assert_eq!(approved.notifications[0].state, NotificationState::Approved);
    }
}
