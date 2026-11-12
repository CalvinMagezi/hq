//! Notifications API over the value bus's `value_items` table: list, approve and dismiss, mark
//! read, put away, clear the Done list and silence a topic, each broadcast to open tabs.

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
    /// For an owner approval: the call verbatim, and whether it loses something (N17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) approval: Option<serde_json::Value>,
    /// For a new sign-in: the device, so "Sign it out" signs out exactly that one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sign_in: Option<serde_json::Value>,
}

/// The approval columns of a joined row: `{tool, args, loses, verb}`, the verb from the tool.
fn approval_from_row(row: &rusqlite::Row) -> rusqlite::Result<Option<serde_json::Value>> {
    let Some(tool) = row.get::<_, Option<String>>(19)? else {
        return Ok(None);
    };
    let args: serde_json::Value = row
        .get::<_, Option<String>>(20)?
        .and_then(|a| serde_json::from_str(&a).ok())
        .unwrap_or_default();
    let verb = hq_core::losing::tool_verb(&tool, &args);
    let site = hq_agent::governance::fetch_site(&tool, &args);
    let thread_id = row
        .get::<_, Option<String>>(18)?
        .as_deref()
        .and_then(web_chat_of)
        .map(str::to_string);
    Ok(Some(serde_json::json!({
        "tool": tool,
        "args": args,
        "loses": verb.is_some(),
        "verb": verb,
        "thread_id": thread_id,
        "site": site,
    })))
}

/// The web chat an owner approval came from, read from its key (`owner-approval:web:<thread>:...`),
/// so that chat shows the same card. `None` for another chat app or a chat not saved yet.
fn web_chat_of(dedup_key: &str) -> Option<&str> {
    let rest = dedup_key.strip_prefix("owner-approval:web:")?;
    let (thread, _) = rest.split_once(':')?;
    (!thread.is_empty() && thread != "new").then_some(thread)
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
    /// The owner has opened it, or marked everything read since it arrived.
    pub(crate) read: bool,
    /// What it is about (`hq_db::inbox_prefs::Topic`), which is what the owner silences.
    pub(crate) topic: String,
    /// Its topic is silenced: it is listed, but nothing alerts and the badge does not count it.
    pub(crate) silenced: bool,
    /// Only the owner can decide it (sign-in, connector, owner approval, escalation, data loss).
    /// Set by the runtime that raised the item; the phone badge counts only these.
    pub(crate) owner_only: bool,
    /// Who it is from: `you`, `helpers` or `system`.
    pub(crate) source_group: String,
    /// Why it left Needs you by itself (`auto_tidy:<rule>`), for the Done list's "No longer
    /// waiting". Absent for every other item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expired_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NotificationsResponse {
    pub(crate) notifications: Vec<NotificationItem>,
    pub(crate) unread_count: usize,
    pub(crate) pending_approvals_count: usize,
    /// Updates that arrived since the owner last read them, silenced topics left out.
    pub(crate) unread_updates_count: usize,
    /// Waiting decisions only the owner can make; what the phone badge counts.
    pub(crate) needs_you_owner_only: usize,
    /// Waiting requests from helpers that the owner's agents are handling.
    pub(crate) needs_you_helpers: usize,
    /// Items the tidy moved out of Needs you that can still be restored.
    pub(crate) expired_count: usize,
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
     v.dedup_key, o.tool, o.args, v.read_at IS NOT NULL, v.owner_only, v.expired_reason \
     FROM value_items v LEFT JOIN dialog_alerts d ON d.item_id = v.id \
     LEFT JOIN owner_approvals o ON o.item_id = v.id";

/// Action-needed items that ask the owner to look, not to decide: approving one would mean
/// nothing, so they offer only Dismiss. Matched on the dedup key's prefix.
const NOTICE_KEYS: &[&str] = &[
    "host-unreachable-",
    "host-lockdown-",
    "disk-watchdog-",
    "self-apply-",
    "bash-sandbox-refusing-",
    "edge-worker-dispatch-",
    "subagent-run-",
];

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
    let dedup_key = row.get::<_, Option<String>>(18)?;
    let notice = is_notice(dedup_key.as_deref());
    // Only HQ's own sign-in item is shown as one: another source with the same key is not.
    let sign_in = dedup_key
        .as_deref()
        .filter(|_| source_task == crate::device_auth::DEVICE_SIGN_IN_SOURCE)
        .and_then(|k| k.strip_prefix(crate::device_auth::SIGN_IN_ITEM_PREFIX))
        .map(|device_id| serde_json::json!({ "device_id": device_id }));
    let actions: &[&str] = match (state == NotificationState::Pending, needs_answer) {
        (false, _) => &[],
        (true, true) if notice => &["dismiss"],
        (true, true) => &["approve", "reject"],
        (true, false) => &["dismiss", "acknowledge"],
    };
    let topic = hq_db::inbox_prefs::Topic::of_source(&source_task).as_str();
    let source_for_group = source_task.clone();
    let owner_only = row.get::<_, i64>(22)? != 0;
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
            approval: approval_from_row(row)?,
            sign_in,
        },
        actions: actions.iter().map(|a| a.to_string()).collect(),
        read: row.get(21)?,
        topic: topic.to_string(),
        silenced: false,
        owner_only,
        source_group: hq_db::inbox_prefs::source_group(&source_for_group, owner_only).to_string(),
        expired_reason: row.get(23)?,
    })
}

/// Collect value-bus items from the SQLite database.
pub(crate) fn scan_value_items(
    db: &hq_db::Database,
    state_filter: Option<&str>,
) -> Result<Vec<NotificationItem>, anyhow::Error> {
    db.with_conn(|conn| {
        // An item cleared off the Done list is on no list.
        let filter = state_where(state_filter);
        let cleared = if filter.is_empty() {
            " WHERE v.cleared_at IS NULL"
        } else {
            " AND v.cleared_at IS NULL"
        };
        let sql = format!("{VALUE_ITEM_COLUMNS}{filter}{cleared}");
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
    let quiet = hq_db::inbox_prefs::Silenced::load(db)?;
    for n in &mut all {
        n.silenced = quiet.covers(&n.metadata.extra["source_task"]);
    }

    let pending = || all.iter().filter(|n| n.state == NotificationState::Pending);
    // The badge: a silenced topic waits in the list without counting.
    let pending_approvals_count = pending()
        .filter(|n| n.kind == NotificationKind::ActionNeeded && !n.silenced)
        .count();
    let unread_updates_count = pending()
        .filter(|n| n.kind == NotificationKind::ValueItem && !n.read && !n.silenced)
        .count();
    let unread_count = pending().count();
    let waiting_decisions =
        || pending().filter(|n| n.kind == NotificationKind::ActionNeeded && !n.silenced);
    let needs_you_owner_only = waiting_decisions().filter(|n| n.owner_only).count();
    let needs_you_helpers = waiting_decisions().filter(|n| !n.owner_only).count();
    let expired_count = restorable_expired_count(db)?;
    all.truncate(limit);

    Ok(NotificationsResponse {
        notifications: all,
        unread_count,
        pending_approvals_count,
        unread_updates_count,
        needs_you_owner_only,
        needs_you_helpers,
        expired_count,
    })
}

/// Items the tidy expired within the restore window and the owner has not cleared.
fn restorable_expired_count(db: &hq_db::Database) -> Result<usize, anyhow::Error> {
    let floor =
        (Utc::now() - chrono::Duration::days(hq_db::inbox_tidy::RESTORE_WINDOW_DAYS)).to_rfc3339();
    db.with_conn(|c| {
        let n: i64 = c.query_row(
            "SELECT COUNT(*) FROM value_items WHERE state = 'expired' AND expired_reason IS NOT NULL \
             AND cleared_at IS NULL AND expired_at >= ?1",
            [floor],
            |r| r.get(0),
        )?;
        Ok(usize::try_from(n).unwrap_or(0))
    })
}

const ALLOW_SITE_ACTION: &str = "approve_site";
const ENGAGEMENT_APPROVED: &str = "approved";

/// What the owner's answer to a fetch request means for its site. When the site is now allowed
/// throughout the chat, the requests waiting on it are answered too, so they stop piling up.
fn note_site_answer(
    db: &hq_db::Database,
    val_id: &str,
    engagement: &str,
    whole_site: bool,
) -> anyhow::Result<()> {
    use hq_agent::governance::{approval_scope, fetch_site, owner_approval_prefix};
    use hq_db::{fetch_sites, owner_approvals};
    let Some((tool, args, key)) = owner_approvals::call_of(db, val_id)? else {
        return Ok(());
    };
    let (Some(site), Some(scope)) = (fetch_site(&tool, &args), approval_scope(&key)) else {
        return Ok(());
    };
    if engagement != ENGAGEMENT_APPROVED {
        return db.with_conn(|c| fetch_sites::forget_in_chat(c, &site, scope));
    }
    let allowed = db.with_conn(|c| {
        if whole_site {
            fetch_sites::allow_whole_site(c, &site, scope)?;
            return Ok(true);
        }
        fetch_sites::record_approval(c, &site, scope)
    })?;
    if !allowed {
        return Ok(());
    }
    for (id, call) in owner_approvals::waiting_calls(db, &owner_approval_prefix(scope), &tool)? {
        if fetch_site(&tool, &call).as_deref() == Some(site.as_str()) {
            record_action(db, &id, ENGAGEMENT_APPROVED)?;
        }
    }
    Ok(())
}

/// The owner allowing HQ Coder's first task from the Inbox. Only this route, which needs the
/// owner's sign-in, can give that yes; no tool can.
fn note_first_task_answer(
    db: &hq_db::Database,
    val_id: &str,
    engagement: &str,
) -> anyhow::Result<()> {
    use hq_tools::github::dispatch::{FIRST_TASK_KEY, kick};
    if engagement != ENGAGEMENT_APPROVED {
        return Ok(());
    }
    let key: Option<String> = db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT dedup_key FROM value_items WHERE id = ?1",
            [val_id],
            |r| r.get(0),
        )?)
    })?;
    if key.as_deref() == Some(FIRST_TASK_KEY) {
        hq_tools::github::app::AppClient::standard()?.confirm_first_task()?;
        kick();
    }
    Ok(())
}

/// Maps a client action to the value item's engagement.
pub(crate) fn action_outcome(action: &str) -> Option<&'static str> {
    match action {
        "approve" | "engage" | ALLOW_SITE_ACTION => Some("approved"),
        "dismiss" | "reject" => Some("dismissed"),
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
    if !matches!(action, "approve" | "engage" | "reject" | ALLOW_SITE_ACTION) {
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
/// `POST /api/notifications/{id}/restore`: the Undo of a clear, within the window.
pub(crate) async fn restore_notification_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(val_id) = id.strip_prefix("val_") else {
        return ApiError::bad_request("unrecognized notification ID format").into_response();
    };
    let window = crate::threads_api::UNDO_WINDOW_SECS;
    match hq_db::value_items::restore_dismissed(&state.db, val_id, window) {
        Ok(true) => {
            state.broadcast(
                &serde_json::json!({"type": "notification:restored", "id": id}).to_string(),
            );
            axum::http::StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => ApiError::Conflict(crate::threads_api::UNDO_TOO_LATE.into()).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// Answers the item whose id is exactly `val_id`. Never a prefix: one approval approves exactly
/// that request, and an approval is signed wherever it lands, so the inbox never shows
/// "approved" for something the approval gate treats as unapproved. An item already answered,
/// expired or spent is left alone and reported as such.
pub(crate) fn record_action(
    db: &hq_db::Database,
    val_id: &str,
    engagement: &str,
) -> anyhow::Result<hq_db::value_items::Answered> {
    hq_db::value_items::answer_by_id(db, val_id, engagement)
}

/// What the person sees when the item was settled before their answer arrived.
fn settled_message(state: hq_core::types::ValueState) -> String {
    use hq_core::types::ValueState;
    let how = match state {
        ValueState::Engaged => "was already approved",
        ValueState::Dismissed => "was already declined or cleared",
        ValueState::Expired => "has expired",
        _ => "is no longer waiting",
    };
    format!("This item {how}; your answer was not recorded.")
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
    let Some(engagement) = action_outcome(&body.action) else {
        return ApiError::bad_request(format!("unknown action '{}' for value item", body.action))
            .into_response();
    };
    if let Err(e) = refuse_unanswered_dialog(&state.db, val_id, &body.action) {
        return e.into_response();
    }
    use hq_db::value_items::Answered;
    let answer = record_action(&state.db, val_id, engagement);
    // A repeat of an answer already given must not count as another approval.
    let first_answer = matches!(answer, Ok(Answered::Recorded));
    match answer {
        Err(e) => {
            warn!(id = %id, action = %body.action, error = %e, "notification action failed");
            return ApiError::internal(e).into_response();
        }
        Ok(Answered::NotFound) => return ApiError::not_found().into_response(),
        Ok(Answered::Settled(s)) => {
            return ApiError::Conflict(settled_message(s)).into_response();
        }
        Ok(Answered::Recorded | Answered::AlreadySo) => {}
    }
    if first_answer {
        let whole_site = body.action == ALLOW_SITE_ACTION;
        if let Err(e) = note_site_answer(&state.db, val_id, engagement, whole_site) {
            warn!(id = %id, error = %e, "recording a site's approval failed");
        }
        if let Err(e) = note_first_task_answer(&state.db, val_id, engagement) {
            warn!(id = %id, error = %e, "recording the first coding task's yes failed");
        }
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

/// Tells every open tab that the inbox changed, with the counts its badge shows.
fn announce_inbox(state: &WsState) {
    state.announce_change(crate::CHANGE_INBOX);
    if let Ok(res) = aggregate_notifications(&state.db, Some("pending"), None, 50) {
        state.broadcast(
            &serde_json::json!({
                "type": "notification:badge_count",
                "unread_count": res.unread_count,
                "pending_approvals_count": res.pending_approvals_count,
                "unread_updates_count": res.unread_updates_count,
            })
            .to_string(),
        );
    }
}

fn done(state: &WsState, body: serde_json::Value) -> Response {
    announce_inbox(state);
    Json(body).into_response()
}

/// `POST /api/notifications/mark-all-read`, the address older apps call for "Clear all
/// updates": puts every update away. A Needs you item waits for its answer.
pub(crate) async fn mark_all_read_handler(state: State<Arc<WsState>>) -> Response {
    archive_all_handler(state).await
}

/// `POST /api/notifications/archive-all`: puts every update away, into Done, and names them so
/// the app can offer Undo. Putting all away never declines a request.
pub(crate) async fn archive_all_handler(State(state): State<Arc<WsState>>) -> Response {
    match hq_db::inbox_prefs::archive_updates(&state.db) {
        Ok(ids) => {
            let ids: Vec<String> = ids.iter().map(|id| format!("val_{id}")).collect();
            done(&state, serde_json::json!({ "ok": true, "ids": ids }))
        }
        Err(e) => {
            warn!(error = %e, "archive_all_handler failed");
            ApiError::internal(e).into_response()
        }
    }
}

/// `POST /api/notifications/read-all`: marks everything waiting as read. Nothing is answered
/// and nothing leaves its list.
pub(crate) async fn read_all_handler(State(state): State<Arc<WsState>>) -> Response {
    match hq_db::inbox_prefs::mark_all_read(&state.db) {
        Ok(n) => done(&state, serde_json::json!({ "ok": true, "read": n })),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `POST /api/notifications/{id}/read`: the owner opened it.
pub(crate) async fn read_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(val_id) = id.strip_prefix("val_") else {
        return ApiError::bad_request("unrecognized notification ID format").into_response();
    };
    match hq_db::inbox_prefs::mark_read(&state.db, val_id) {
        Ok(true) => done(&state, serde_json::json!({ "ok": true })),
        Ok(false) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `POST /api/notifications/clear-done`: empties the Done list.
pub(crate) async fn clear_done_handler(State(state): State<Arc<WsState>>) -> Response {
    match hq_db::inbox_prefs::clear_done(&state.db) {
        Ok(n) => done(&state, serde_json::json!({ "ok": true, "cleared": n })),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RestoreManyRequest {
    pub(crate) ids: Vec<String>,
}

/// Most ids one Undo may name; "Clear all updates" on a long-neglected inbox stays under it.
const MAX_RESTORE: usize = 1000;

/// `POST /api/notifications/restore`: the Undo of putting all updates away, within the window.
/// Answers how many came back; one that was answered since, or is too old, stays where it is.
pub(crate) async fn restore_many_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<RestoreManyRequest>,
) -> Response {
    if body.ids.len() > MAX_RESTORE {
        return ApiError::bad_request("too many items to restore at once").into_response();
    }
    let window = crate::threads_api::UNDO_WINDOW_SECS;
    let mut restored = 0;
    for id in &body.ids {
        let Some(val_id) = id.strip_prefix("val_") else {
            return ApiError::bad_request("unrecognized notification ID format").into_response();
        };
        match hq_db::value_items::restore_dismissed(&state.db, val_id, window) {
            Ok(true) => restored += 1,
            Ok(false) => {}
            Err(e) => return ApiError::internal(e).into_response(),
        }
    }
    if restored == 0 && !body.ids.is_empty() {
        return ApiError::Conflict(crate::threads_api::UNDO_TOO_LATE.into()).into_response();
    }
    done(
        &state,
        serde_json::json!({ "ok": true, "restored": restored }),
    )
}

fn silences_body(db: &hq_db::Database) -> anyhow::Result<serde_json::Value> {
    use hq_db::inbox_prefs::{ALL, Topic, silences};
    let active = silences(db, Utc::now())?;
    let until_of = |name: &str| active.iter().find(|s| s.topic == name);
    let row = |name: &str| {
        let s = until_of(name);
        serde_json::json!({
            "topic": name,
            "silenced": s.is_some(),
            "until": s.and_then(|s| s.until).map(|u| u.to_rfc3339()),
        })
    };
    Ok(serde_json::json!({
        "pause": row(ALL),
        "topics": Topic::SILENCEABLE.iter().map(|t| row(t.as_str())).collect::<Vec<_>>(),
    }))
}

/// `GET /api/notifications/silences`: the pause and every topic that can be silenced.
pub(crate) async fn silences_handler(State(state): State<Arc<WsState>>) -> Response {
    match silences_body(&state.db) {
        Ok(body) => Json(body).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SilenceRequest {
    /// A topic's name, or `all` for the pause.
    pub(crate) topic: String,
    /// How long, in minutes; absent until it is turned back on.
    pub(crate) minutes: Option<i64>,
}

/// A week: longer than that is "until I turn it back on".
const MAX_SILENCE_MINUTES: i64 = 7 * 24 * 60;

/// `POST /api/notifications/silences`: silences a topic, or pauses them all. What HQ itself
/// asks the owner is not a topic and is refused here.
pub(crate) async fn silence_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<SilenceRequest>,
) -> Response {
    let until = match body.minutes {
        None => None,
        Some(m) if (1..=MAX_SILENCE_MINUTES).contains(&m) => {
            Some(Utc::now() + chrono::Duration::minutes(m))
        }
        Some(_) => return ApiError::bad_request("minutes is out of range").into_response(),
    };
    match hq_db::inbox_prefs::silence(&state.db, &body.topic, until) {
        Ok(true) => {}
        Ok(false) => return ApiError::bad_request("this cannot be silenced").into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    }
    info!(topic = %body.topic, minutes = ?body.minutes, "inbox topic silenced");
    match silences_body(&state.db) {
        Ok(body) => done(&state, body),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `DELETE /api/notifications/silences/{topic}`: turns a topic, or the pause, back on.
pub(crate) async fn unsilence_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(topic): AxumPath<String>,
) -> Response {
    if let Err(e) = hq_db::inbox_prefs::unsilence(&state.db, &topic) {
        return ApiError::internal(e).into_response();
    }
    match silences_body(&state.db) {
        Ok(body) => done(&state, body),
        Err(e) => ApiError::internal(e).into_response(),
    }
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

    #[test]
    fn an_approval_names_the_web_chat_it_came_from() {
        assert_eq!(
            web_chat_of("owner-approval:web:t-42:bash:abc"),
            Some("t-42")
        );
        assert_eq!(web_chat_of("owner-approval:web:new:bash:abc"), None);
        assert_eq!(
            web_chat_of("owner-approval:relay:telegram:9:bash:abc"),
            None
        );
        assert_eq!(web_chat_of("disk-watchdog-2026-11-05"), None);
    }

    #[test]
    fn counts_split_owner_only_decisions_from_helper_requests_and_list_the_tidied() {
        use hq_core::types::{ValueItem, ValueKind};
        let db = hq_db::Database::open_memory().unwrap();
        let put = |source: &str, key: &str, owner_only: bool| {
            let item = ValueItem::new(source, ValueKind::ActionNeeded, "t", "b")
                .with_dedup_key(key)
                .owner_only(owner_only);
            hq_db::value_items::insert(&db, &item).unwrap();
            item.id
        };
        put("device-sign-in", "sign-in-1", true);
        put("session-supervisor", "session-blocked-hs-1-1", false);
        put("session-supervisor", "session-blocked-hs-2-1", false);
        let gone = put("session-supervisor", "session-blocked-hs-3-1", false);
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET state = 'expired', expired_reason = 'auto_tidy:session_ended', \
                 expired_at = ?2 WHERE id = ?1",
                rusqlite::params![gone, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
        .unwrap();

        let res = aggregate_notifications(&db, None, None, 50).unwrap();
        assert_eq!(res.needs_you_owner_only, 1);
        assert_eq!(res.needs_you_helpers, 2);
        assert_eq!(res.pending_approvals_count, 3, "the old count is unchanged");
        assert_eq!(res.expired_count, 1);
        let tidied = res
            .notifications
            .iter()
            .find(|n| n.id == format!("val_{gone}"))
            .unwrap();
        assert_eq!(
            tidied.expired_reason.as_deref(),
            Some("auto_tidy:session_ended")
        );
        assert_eq!(tidied.source_group, "helpers");
        let sign_in = res.notifications.iter().find(|n| n.owner_only).unwrap();
        assert_eq!(sign_in.source_group, "you");
    }

    #[test]
    fn a_notice_to_look_at_offers_only_dismiss() {
        let db = hq_db::Database::open_memory().unwrap();
        let item = hq_core::types::ValueItem::new(
            "disk_watchdog",
            hq_core::types::ValueKind::ActionNeeded,
            "Disk watchdog: threshold(s) breached",
            "Disk 96% used",
        )
        .with_dedup_key("disk-watchdog-2026-11-05");
        hq_db::value_items::insert(&db, &item).unwrap();
        let items = scan_value_items(&db, Some("pending")).unwrap();
        assert_eq!(items[0].actions, ["dismiss"]);
    }

    #[test]
    fn an_owner_approval_quotes_its_call_and_marks_a_losing_one() {
        let db = hq_db::Database::open_memory().unwrap();
        let ask = |tool: &str, args: serde_json::Value| {
            let item = hq_core::types::ValueItem::new(
                "owner-approval",
                hq_core::types::ValueKind::ActionNeeded,
                format!("HQ wants to run {tool}"),
                "Approve to let it run once.",
            );
            hq_db::owner_approvals::record(&db, &item.id, tool, &args).unwrap();
            hq_db::value_items::insert(&db, &item).unwrap();
            format!("val_{}", item.id)
        };
        let delete = ask("task_delete", serde_json::json!({"id": "t-1"}));
        let list = ask("bash", serde_json::json!({"command": "ls -la"}));
        let items = scan_value_items(&db, Some("pending")).unwrap();
        let approval = |id: &str| {
            items
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .metadata
                .approval
                .clone()
                .unwrap()
        };
        let d = approval(&delete);
        assert!(
            d["thread_id"].is_null(),
            "an item with no web chat key names no chat"
        );
        assert_eq!(d["tool"], "task_delete");
        assert_eq!(d["args"]["id"], "t-1");
        assert_eq!(d["loses"], true);
        assert_eq!(d["verb"], "delete");
        let l = approval(&list);
        assert_eq!(l["loses"], false);
        assert!(l["verb"].is_null());
    }

    #[test]
    fn a_helpers_dialog_that_deletes_is_marked_losing() {
        let db = hq_db::Database::open_memory().unwrap();
        let item = hq_core::types::ValueItem::new(
            "session-supervisor",
            hq_core::types::ValueKind::ActionNeeded,
            "waiting",
            "blocked",
        );
        hq_db::value_items::insert(&db, &item).unwrap();
        let options = vec![hq_db::dialog_alerts::DialogOption {
            id: 1,
            label: "Yes".into(),
            remember: false,
            typed: false,
        }];
        hq_db::dialog_alerts::record(
            &db,
            &hq_db::dialog_alerts::NewDialogAlert {
                item_id: &item.id,
                session_id: "hs-1",
                pending_seq: 3,
                kind: "command",
                question: Some("Run this?"),
                command: Some("git clean -fdx"),
                options: &options,
            },
        )
        .unwrap();
        let items = scan_value_items(&db, Some("pending")).unwrap();
        let dialog = items[0].metadata.dialog.clone().unwrap();
        assert_eq!(dialog["loses"], true);
        assert_eq!(dialog["verb"], "delete");
        assert!(items[0].metadata.approval.is_none());
    }

    fn fetch_request(db: &hq_db::Database, chat: &str, url: &str, digest: &str) -> String {
        let item = hq_core::types::ValueItem::new(
            "owner-approval",
            hq_core::types::ValueKind::ActionNeeded,
            "HQ wants to run web_fetch",
            "b",
        )
        .with_dedup_key(format!("owner-approval:{chat}:web_fetch:{digest}"));
        hq_db::owner_approvals::record(db, &item.id, "web_fetch", &serde_json::json!({"url": url}))
            .unwrap();
        hq_db::value_items::insert(db, &item).unwrap();
        item.id
    }

    fn still_waiting(db: &hq_db::Database, id: &str) -> bool {
        scan_value_items(db, Some("pending"))
            .unwrap()
            .iter()
            .any(|i| i.id == format!("val_{id}"))
    }

    #[test]
    fn allowing_a_site_answers_only_that_sites_short_requests_in_that_chat() {
        let db = hq_db::Database::open_memory().unwrap();
        let chat = "web:t1";
        let asked = fetch_request(&db, chat, "https://a.example/one", "d1");
        let same = fetch_request(&db, chat, "https://www.a.example/two", "d2");
        let other = fetch_request(&db, chat, "https://b.example/", "d3");
        let long = format!("https://a.example/x?d={}", "a".repeat(250));
        let long = fetch_request(&db, chat, &long, "d4");
        let elsewhere = fetch_request(&db, "web:t2", "https://a.example/three", "d5");

        record_action(&db, &asked, ENGAGEMENT_APPROVED).unwrap();
        note_site_answer(&db, &asked, ENGAGEMENT_APPROVED, true).unwrap();

        assert!(!still_waiting(&db, &same), "the same site clears");
        assert!(still_waiting(&db, &other), "another site still asks");
        assert!(still_waiting(&db, &long), "a long address still asks");
        assert!(still_waiting(&db, &elsewhere), "another chat still asks");
    }

    #[test]
    fn a_plain_approval_widens_to_the_site_only_on_the_third() {
        let db = hq_db::Database::open_memory().unwrap();
        for n in 1..=2 {
            let id = fetch_request(
                &db,
                "web:t1",
                &format!("https://a.example/{n}"),
                &format!("d{n}"),
            );
            record_action(&db, &id, ENGAGEMENT_APPROVED).unwrap();
            note_site_answer(&db, &id, ENGAGEMENT_APPROVED, false).unwrap();
        }
        let waiting = fetch_request(&db, "web:t1", "https://a.example/next", "dn");
        assert!(still_waiting(&db, &waiting));
        let third = fetch_request(&db, "web:t1", "https://a.example/3", "d3");
        record_action(&db, &third, ENGAGEMENT_APPROVED).unwrap();
        note_site_answer(&db, &third, ENGAGEMENT_APPROVED, false).unwrap();
        assert!(!still_waiting(&db, &waiting));
    }

    /// Only HQ's own sign-in item is shown as a new sign-in; an item from another source with the
    /// same kind of key is an ordinary one, so it cannot offer "Sign it out" for a real device.
    #[test]
    fn only_hqs_own_sign_in_item_reads_as_one() {
        let db = hq_db::Database::open_memory().unwrap();
        let item = |source: &str| {
            hq_core::types::ValueItem::new(
                source,
                hq_core::types::ValueKind::ActionNeeded,
                "A new sign-in to your HQ",
                "b",
            )
            .with_dedup_key(format!("{}dev1", crate::device_auth::SIGN_IN_ITEM_PREFIX))
        };
        let real = item(crate::device_auth::DEVICE_SIGN_IN_SOURCE);
        let forged = item(hq_tools::notify::NOTIFY_SOURCE);
        hq_db::value_items::insert(&db, &real).unwrap();
        hq_db::value_items::insert(&db, &forged).unwrap();
        let items = scan_value_items(&db, Some("pending")).unwrap();
        let sign_in = |id: &str| {
            items
                .iter()
                .find(|n| n.id == format!("val_{id}"))
                .unwrap()
                .metadata
                .sign_in
                .clone()
        };
        assert_eq!(
            sign_in(&real.id),
            Some(serde_json::json!({ "device_id": "dev1" }))
        );
        assert_eq!(sign_in(&forged.id), None);
        assert!(
            hq_tools::notify::RESERVED_DEDUP_PREFIXES
                .contains(&crate::device_auth::SIGN_IN_ITEM_PREFIX),
            "notify_owner refuses this key"
        );
    }

    #[tokio::test]
    async fn clearing_all_updates_leaves_every_needs_you_item_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let ask = hq_core::types::ValueItem::new(
            "owner-approval",
            hq_core::types::ValueKind::ActionNeeded,
            "HQ wants to run bash",
            "b",
        );
        let news = hq_core::types::ValueItem::new(
            "notify_owner",
            hq_core::types::ValueKind::Fyi,
            "Deploy finished",
            "b",
        );
        hq_db::value_items::insert(&state.db, &ask).unwrap();
        hq_db::value_items::insert(&state.db, &news).unwrap();
        assert_eq!(
            mark_all_read_handler(State(state.clone())).await.status(),
            200
        );
        let pending = scan_value_items(&state.db, Some("pending")).unwrap();
        let ids: Vec<&str> = pending.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, [format!("val_{}", ask.id)]);
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

    /// One approve approves exactly the item it names. A short id used to approve and sign every
    /// delivered item sharing that prefix, owner-approval requests included; an item that was
    /// not delivered was marked approved without a signature; and a spent approval came back.
    #[tokio::test]
    async fn approving_by_id_approves_exactly_that_item_and_signs_it() {
        use hq_core::types::{ValueItem, ValueKind};
        use hq_db::value_items as vi;
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let ask = |dedup: &str, id: &str| {
            let mut item =
                ValueItem::new("owner-approval", ValueKind::ActionNeeded, "Run it?", "b")
                    .with_dedup_key(dedup);
            item.id = id.into();
            vi::insert(&state.db, &item).unwrap();
            vi::set_delivered(&state.db, id).unwrap();
        };
        ask("owner:a", "feed0001-aaaa");
        ask("owner:b", "feed0002-bbbb");
        let act = |id: &str, action: &str| {
            let body = NotificationActionRequest {
                action: action.into(),
            };
            notification_action_handler(State(state.clone()), AxumPath(id.into()), Json(body))
        };

        assert_eq!(act("val_feed", "approve").await.status(), 404);
        assert!(!vi::is_signed_approval(&state.db, "owner:a").unwrap());
        assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());

        assert_eq!(act("val_feed0001-aaaa", "approve").await.status(), 200);
        assert!(vi::is_signed_approval(&state.db, "owner:a").unwrap());
        assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());
        assert_eq!(
            act("val_feed0001-aaaa", "approve").await.status(),
            200,
            "the same answer again changes nothing"
        );

        // Spent by the one call it approved, it cannot be approved back into a second run.
        assert!(vi::consume_approval(&state.db, "owner:a").unwrap());
        assert_eq!(act("val_feed0001-aaaa", "approve").await.status(), 409);
        assert!(!vi::consume_approval(&state.db, "owner:a").unwrap());
        assert_eq!(act("val_feed0001-aaaa", "dismiss").await.status(), 409);

        // Never delivered: approved and signed all the same, or the gate would not run it.
        let mut waiting = ValueItem::new("owner-approval", ValueKind::ActionNeeded, "Run?", "b")
            .with_dedup_key("owner:c");
        waiting.id = "feed0003-cccc".into();
        vi::insert(&state.db, &waiting).unwrap();
        assert_eq!(act("val_feed0003-cccc", "approve").await.status(), 200);
        assert!(vi::is_signed_approval(&state.db, "owner:c").unwrap());

        assert_eq!(act("val_feed0002-bbbb", "dismiss").await.status(), 200);
        assert_eq!(
            act("val_feed0002-bbbb", "approve").await.status(),
            409,
            "a declined request is not approved afterwards"
        );
        assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());
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

    fn insert(state: &WsState, source: &str, kind: hq_core::types::ValueKind) -> String {
        let item = hq_core::types::ValueItem::new(source, kind, "t", "b");
        hq_db::value_items::insert(&state.db, &item).unwrap();
        format!("val_{}", item.id)
    }

    async fn json_of(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn reading_keeps_an_update_in_its_list_and_drops_it_from_the_unread_count() {
        use hq_core::types::ValueKind;
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let news = insert(&state, "agent_notify", ValueKind::Fyi);
        insert(&state, "disk-watchdog", ValueKind::Fyi);
        let before = aggregate_notifications(&state.db, Some("pending"), None, 50).unwrap();
        assert_eq!(before.unread_updates_count, 2);

        let res = read_handler(State(state.clone()), AxumPath(news.clone())).await;
        assert_eq!(res.status(), 200);
        let after = aggregate_notifications(&state.db, Some("pending"), None, 50).unwrap();
        assert_eq!(after.unread_updates_count, 1);
        assert_eq!(after.notifications.len(), 2, "read is not put away");
        assert!(after.notifications.iter().any(|n| n.id == news && n.read));

        assert_eq!(read_all_handler(State(state.clone())).await.status(), 200);
        let after = aggregate_notifications(&state.db, Some("pending"), None, 50).unwrap();
        assert_eq!(
            (after.unread_updates_count, after.notifications.len()),
            (0, 2)
        );
    }

    #[tokio::test]
    async fn putting_all_away_can_be_undone_and_done_can_be_cleared() {
        use hq_core::types::ValueKind;
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let news = insert(&state, "agent_notify", ValueKind::Fyi);
        let ask = insert(&state, "owner-approval", ValueKind::ActionNeeded);

        let body = json_of(archive_all_handler(State(state.clone())).await).await;
        assert_eq!(body["ids"], serde_json::json!([news]));
        let pending = scan_value_items(&state.db, Some("pending")).unwrap();
        assert_eq!(pending.iter().map(|n| &n.id).collect::<Vec<_>>(), [&ask]);

        let undo = RestoreManyRequest {
            ids: vec![news.clone()],
        };
        let res = restore_many_handler(State(state.clone()), Json(undo)).await;
        assert_eq!(json_of(res).await["restored"], 1);
        assert_eq!(
            scan_value_items(&state.db, Some("pending")).unwrap().len(),
            2
        );
        // A request can never be brought back by Undo: it was never put away.
        let undo = RestoreManyRequest {
            ids: vec![ask.clone()],
        };
        let res = restore_many_handler(State(state.clone()), Json(undo)).await;
        assert_eq!(res.status(), 409);

        archive_all_handler(State(state.clone())).await;
        assert_eq!(
            scan_value_items(&state.db, Some("history")).unwrap().len(),
            1
        );
        assert_eq!(clear_done_handler(State(state.clone())).await.status(), 200);
        assert!(
            scan_value_items(&state.db, Some("history"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            scan_value_items(&state.db, None).unwrap().len(),
            1,
            "the request still waits"
        );
    }

    #[tokio::test]
    async fn a_silenced_topic_is_listed_but_not_counted_and_hq_is_never_silenced() {
        use hq_core::types::ValueKind;
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let helper = insert(&state, "session-supervisor", ValueKind::ActionNeeded);
        insert(&state, "owner-approval", ValueKind::ActionNeeded);

        let ask = |topic: &str, minutes| SilenceRequest {
            topic: topic.into(),
            minutes,
        };
        let res = silence_handler(State(state.clone()), Json(ask("hq", None))).await;
        assert_eq!(res.status(), 400);
        let res = silence_handler(State(state.clone()), Json(ask("helpers", Some(0)))).await;
        assert_eq!(res.status(), 400);
        let res = silence_handler(State(state.clone()), Json(ask("helpers", Some(60)))).await;
        let body = json_of(res).await;
        let helpers = body["topics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["topic"] == "helpers");
        assert_eq!(helpers.unwrap()["silenced"], true);
        assert!(helpers.unwrap()["until"].is_string());

        let list = aggregate_notifications(&state.db, Some("pending"), None, 50).unwrap();
        assert_eq!(
            list.notifications.len(),
            2,
            "silenced items are still listed"
        );
        assert_eq!(
            list.pending_approvals_count, 1,
            "only HQ's own request counts"
        );
        let item = list.notifications.iter().find(|n| n.id == helper).unwrap();
        assert!(item.silenced && item.topic == "helpers");

        let res = unsilence_handler(State(state.clone()), AxumPath("helpers".into())).await;
        assert_eq!(res.status(), 200);
        let list = aggregate_notifications(&state.db, Some("pending"), None, 50).unwrap();
        assert_eq!(list.pending_approvals_count, 2);
    }
}
