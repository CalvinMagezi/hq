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
    /// For a blocked session's alert: the dialog it is about and the buttons it may have,
    /// answered through `/api/notifications/{id}/answer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) dialog: Option<serde_json::Value>,
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

/// Each item with the dialog it is about, when it is a blocked session's alert (`dialog_alerts`).
const VALUE_ITEM_COLUMNS: &str = "SELECT v.id, v.source_task, v.kind, v.title, v.body, v.artifact_path, v.score, v.state, v.created_at, v.link, \
     d.session_id, d.pending_seq, d.kind, d.question, d.command, d.options, d.approve_option, d.answered_at IS NOT NULL, \
     v.dedup_key \
     FROM value_items v LEFT JOIN dialog_alerts d ON d.item_id = v.id";

/// Action-needed items that ask the owner to look, not to decide: approving one would mean
/// nothing, so they offer only Dismiss. Matched on the dedup key's prefix.
const NOTICE_KEYS: &[&str] = &["host-unreachable-"];

fn is_notice(dedup_key: Option<&str>) -> bool {
    dedup_key.is_some_and(|k| NOTICE_KEYS.iter().any(|p| k.starts_with(p)))
}

fn state_where(state_filter: Option<&str>) -> &'static str {
    match state_filter {
        Some("pending") => " WHERE v.state IN ('pending', 'delivered')",
        Some("approved") => " WHERE v.state = 'engaged'",
        Some("dismissed") => " WHERE v.state = 'dismissed'",
        Some("resolved") | Some("history") => {
            " WHERE v.state IN ('engaged', 'dismissed', 'expired')"
        }
        _ => "",
    }
}

/// The dialog columns of a joined row, when the item has one.
fn dialog_from_row(
    row: &rusqlite::Row,
    item_id: &str,
) -> rusqlite::Result<Option<serde_json::Value>> {
    let Some(session_id) = row.get::<_, Option<String>>(10)? else {
        return Ok(None);
    };
    let options: String = row.get(15)?;
    let alert = hq_db::dialog_alerts::DialogAlert {
        item_id: item_id.to_string(),
        session_id,
        pending_seq: row.get::<_, i64>(11)? as u64,
        kind: row.get(12)?,
        question: row.get(13)?,
        command: row.get(14)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        approve_option: row.get::<_, Option<i64>>(16)?.map(|a| a as u32),
        answered: row.get(17)?,
    };
    Ok(Some(crate::dialog_answer::dialog_view(&alert)))
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
    let notice = is_notice(row.get::<_, Option<String>>(18)?.as_deref());
    let actions: &[&str] = match (state == NotificationState::Pending, needs_answer) {
        (false, _) => &[],
        (true, true) if notice => &["dismiss"],
        (true, true) => &["approve", "reject"],
        (true, false) => &["dismiss", "acknowledge"],
    };
    let extra = BTreeMap::from([
        ("source_task".to_string(), source_task),
        ("val_kind".to_string(), kind_str),
    ]);
    let dialog = dialog_from_row(row, &id)?;
    // A question from an agent is answered through its dialog, never by approving the notice.
    let awaits_dialog = dialog
        .as_ref()
        .is_some_and(|d| d["answered"] == serde_json::Value::Bool(false));
    let actions: &[&str] = if awaits_dialog && !actions.is_empty() {
        &["dismiss"]
    } else {
        actions
    };
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
            dialog,
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
pub(crate) fn action_outcome(action: &str) -> Option<(&'static str, &'static str)> {
    match action {
        "approve" | "engage" => Some(("approved", "engaged")),
        "dismiss" | "reject" => Some(("dismissed", "dismissed")),
        _ => None,
    }
}

/// Approving or rejecting the notice of an agent's open dialog would only clear the notice and
/// leave the agent waiting, so it is refused; the dialog's own answer route answers it.
/// Dismissing the notice stays allowed.
fn refuse_unanswered_dialog(
    db: &hq_db::Database,
    val_id: &str,
    action: &str,
) -> Result<(), ApiError> {
    if !matches!(action, "approve" | "engage" | "reject") {
        return Ok(());
    }
    let open = hq_db::dialog_alerts::get(db, val_id)
        .map_err(ApiError::internal)?
        .is_some_and(|a| !a.answered);
    if open {
        return Err(ApiError::Conflict(DIALOG_NOT_ANSWERED.into()));
    }
    Ok(())
}

const DIALOG_NOT_ANSWERED: &str = "This is an agent waiting at a question. Answer it with its own buttons or on the agent's page; approving the notice would not answer the agent.";

/// Returns false when no value item has that id.
pub(crate) fn record_action(
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
    if let Err(e) = refuse_unanswered_dialog(&state.db, val_id, &body.action) {
        return e.into_response();
    }
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
        assert!(state_where(Some("pending")).starts_with(" WHERE v.state IN"));
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

    #[test]
    fn a_host_that_is_not_answering_offers_nothing_to_approve() {
        let db = hq_db::Database::open_memory().unwrap();
        let item = hq_core::types::ValueItem::new(
            "session-supervisor",
            hq_core::types::ValueKind::ActionNeeded,
            "Host 'laptop' is not answering",
            "Host laptop has not answered for 5 minutes.",
        )
        .with_dedup_key("host-unreachable-laptop-1000");
        hq_db::value_items::emit(&db, &item).unwrap();
        let items = scan_value_items(&db, Some("pending")).unwrap();
        assert_eq!(items[0].kind, NotificationKind::ActionNeeded);
        assert_eq!(items[0].actions, ["dismiss"]);
    }

    #[test]
    fn a_blocked_sessions_alert_lists_its_dialog_and_its_one_button() {
        let db = hq_db::Database::open_memory().unwrap();
        let item = hq_core::types::ValueItem::new(
            "session-supervisor",
            hq_core::types::ValueKind::ActionNeeded,
            "Harness session 'copilot' is waiting for you",
            "Session hs-1 on laptop is blocked on a prompt or approval",
        );
        hq_db::value_items::insert(&db, &item).unwrap();
        let plain = hq_core::types::ValueItem::new(
            "notify_owner",
            hq_core::types::ValueKind::Fyi,
            "Deploy finished",
            "v1.2 is live",
        );
        hq_db::value_items::insert(&db, &plain).unwrap();
        let options = vec![
            hq_db::dialog_alerts::DialogOption {
                id: 1,
                label: "Yes".into(),
                remember: false,
                typed: false,
            },
            hq_db::dialog_alerts::DialogOption {
                id: 2,
                label: "Yes, and remember this path".into(),
                remember: true,
                typed: false,
            },
        ];
        hq_db::dialog_alerts::record(
            &db,
            &hq_db::dialog_alerts::NewDialogAlert {
                item_id: &item.id,
                session_id: "hs-1",
                pending_seq: 7,
                kind: "path",
                question: Some("Do you want to allow this?"),
                command: Some("/tmp"),
                options: &options,
            },
        )
        .unwrap();
        let items = scan_value_items(&db, Some("pending")).unwrap();
        let alert = items
            .iter()
            .find(|n| n.id == format!("val_{}", item.id))
            .unwrap();
        let dialog = alert
            .metadata
            .dialog
            .as_ref()
            .expect("the alert carries its dialog");
        assert_eq!(dialog["seq"], 7);
        assert_eq!(dialog["approve"]["label"], "Yes");
        assert_eq!(dialog["options"][1]["remember"], true);
        let news = items
            .iter()
            .find(|n| n.id == format!("val_{}", plain.id))
            .unwrap();
        assert!(news.metadata.dialog.is_none());
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

    #[tokio::test]
    async fn an_open_dialog_is_not_approved_through_its_notice() {
        // Approving the notice of a question once left the agent waiting with the alert gone.
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let item = hq_core::types::ValueItem::new(
            "session-supervisor",
            hq_core::types::ValueKind::ActionNeeded,
            "Harness session 'claude' is waiting for you",
            "Session hs-1 on laptop is blocked on a prompt or approval",
        );
        hq_db::value_items::insert(&state.db, &item).unwrap();
        let options = ["Red", "Blue"].map(|label| hq_db::dialog_alerts::DialogOption {
            id: if label == "Red" { 1 } else { 2 },
            label: label.into(),
            remember: false,
            typed: false,
        });
        hq_db::dialog_alerts::record(
            &state.db,
            &hq_db::dialog_alerts::NewDialogAlert {
                item_id: &item.id,
                session_id: "hs-1",
                pending_seq: 7,
                kind: "other",
                question: Some("Which colour should the test use?"),
                command: None,
                options: &options,
            },
        )
        .unwrap();
        let id = format!("val_{}", item.id);
        let listed = scan_value_items(&state.db, Some("pending")).unwrap();
        let notice = listed.iter().find(|n| n.id == id).unwrap();
        assert_eq!(notice.actions, ["dismiss"], "only its dialog answers it");

        let act = |action: &str| {
            let body = NotificationActionRequest {
                action: action.into(),
            };
            notification_action_handler(State(state.clone()), AxumPath(id.clone()), Json(body))
        };
        for action in ["approve", "engage", "reject"] {
            assert_eq!(act(action).await.status(), 409, "{action}");
        }
        assert_eq!(
            scan_value_items(&state.db, Some("pending")).unwrap().len(),
            1,
            "a refused action leaves the notice in place"
        );
        assert_eq!(act("dismiss").await.status(), 200);
    }
}
