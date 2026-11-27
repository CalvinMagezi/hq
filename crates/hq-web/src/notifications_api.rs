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
    let consequence = hq_core::losing::consequence(&tool, &args, site.as_deref());
    let dedup_key = row.get::<_, Option<String>>(18)?;
    let thread_id = dedup_key
        .as_deref()
        .and_then(web_chat_of)
        .map(str::to_string);
    let scope = dedup_key
        .as_deref()
        .and_then(hq_agent::governance::approval_scope)
        .map(str::to_string);
    let folder = hq_agent::governance::folder_scope::candidate_folder(&tool, &args, None)
        .map(|p| p.to_string_lossy().into_owned());
    Ok(Some(serde_json::json!({
        "scope": scope,
        "folder": folder,
        "tool": tool,
        "args": args,
        "loses": verb.is_some(),
        "verb": verb,
        "thread_id": thread_id,
        "site": site,
        "consequence": consequence,
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
    /// A dismiss-only notice: it says something happened and asks for no decision, so it never
    /// counts toward Needs you unless the runtime marked it owner-only.
    #[serde(skip)]
    pub(crate) notice: bool,
    /// Who it is from: `you`, `helpers` or `system`.
    pub(crate) source_group: String,
    /// `owner` or `agent` (`hq_db::inbox_prefs::audience`): what decides the default list, the
    /// badge and push.
    pub(crate) audience: &'static str,
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
    /// `1` lists helper and agent traffic too (the Helpers and Tasks pages); the default list
    /// and the badge hold only what is for the owner.
    pub(crate) include_agent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NotificationActionRequest {
    pub(crate) action: String,
    /// For `allow_starter` on a helper's access request: which starter set to allow.
    #[serde(default)]
    pub(crate) starter: Option<String>,
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

/// A pending action-needed item counts toward Needs you only when someone must decide it.
fn needs_a_human(n: &NotificationItem) -> bool {
    n.owner_only || !n.notice
}

/// A sub-agent run's alert is read in plain words whatever text the row was stored with: rows
/// written before the wording changed carry a run id, a role and a tool name.
fn owner_wording(
    source: &str,
    dedup_key: Option<&str>,
    title: String,
    body: String,
) -> (String, String) {
    let delegated =
        source == "subagent_runs" && dedup_key.is_some_and(|k| k.starts_with("subagent-run-"));
    if delegated {
        (
            "A background task did not finish".to_string(),
            "A background task did not finish. Open the chat to see what is missing.".to_string(),
        )
    } else {
        (title, body)
    }
}

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
    // A Later offer's item can bring the offer back; no other update has an `open` action.
    let later_offer = source_task == hq_core::types::SCHEDULED_OFFER_SOURCE
        && dedup_key
            .as_deref()
            .is_some_and(|k| k.starts_with(hq_db::offers::LATER_ITEM_PREFIX));
    let actions: &[&str] = match (state == NotificationState::Pending, needs_answer) {
        (false, _) => &[],
        (true, false) if later_offer => &["open", "dismiss"],
        (true, true) if notice => &["dismiss"],
        (true, true) => &["approve", "reject"],
        (true, false) => &["dismiss", "acknowledge"],
    };
    let (title, description) =
        owner_wording(&source_task, dedup_key.as_deref(), row.get(3)?, row.get(4)?);
    let topic = hq_db::inbox_prefs::Topic::of_source(&source_task).as_str();
    let source_for_group = source_task.clone();
    let owner_only = row.get::<_, i64>(22)? != 0;
    let mut extra = BTreeMap::from([
        ("source_task".to_string(), source_task),
        ("val_kind".to_string(), kind_str),
    ]);
    if later_offer {
        extra.insert("open_label".to_string(), LATER_OPEN_LABEL.to_string());
    }
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
        title,
        description,
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
        notice,
        source_group: hq_db::inbox_prefs::source_group(&source_for_group, owner_only).to_string(),
        audience: match hq_db::inbox_prefs::audience(&source_for_group, owner_only, needs_answer) {
            hq_db::inbox_prefs::Audience::Owner => "owner",
            hq_db::inbox_prefs::Audience::Agent => "agent",
        },
        expired_reason: row.get(23)?,
    })
}

/// Collect value-bus items from the SQLite database, each with the extra facts its card shows.
#[cfg(test)]
pub(crate) fn scan_value_items(
    db: &hq_db::Database,
    state_filter: Option<&str>,
) -> Result<Vec<NotificationItem>, anyhow::Error> {
    enrich_items(db, scan_value_rows(db, state_filter)?)
}

/// Adds the per-card lookups (`with_site_approvals`, `with_folder_nudge`) to `items`. Each is one
/// or two queries, so a list that will be cut down does it after the cut.
fn enrich_items(
    db: &hq_db::Database,
    items: Vec<NotificationItem>,
) -> Result<Vec<NotificationItem>, anyhow::Error> {
    db.with_conn(|conn| {
        Ok(items
            .into_iter()
            .map(|item| with_folder_nudge(conn, with_site_approvals(conn, item)))
            .collect())
    })
}

/// The value-bus rows as stored: no per-card lookups.
fn scan_value_rows(
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
        Ok(stmt
            .query_map([], notification_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Adds `recent_requests` to an approval item a folder allow could cover: how many approvals for
/// commands and edits its chat has raised lately, so the card can offer the folder allow or a
/// helper from the third one on. A read that fails leaves the item without the field.
fn with_folder_nudge(conn: &rusqlite::Connection, mut item: NotificationItem) -> NotificationItem {
    let Some(approval) = item.metadata.approval.as_mut() else {
        return item;
    };
    let (Some(scope), Some(tool)) = (
        approval["scope"].as_str().map(str::to_string),
        approval["tool"].as_str().map(str::to_string),
    ) else {
        return item;
    };
    if !hq_agent::governance::STREAK_TOOLS.contains(&tool.as_str()) {
        return item;
    }
    let prefix = hq_agent::governance::owner_approval_prefix(&scope);
    if let Ok(n) = hq_db::owner_folder_grants::recent_requests_in(
        conn,
        &prefix,
        hq_agent::governance::STREAK_TOOLS,
        hq_agent::governance::STREAK_WINDOW_SECS,
    ) {
        approval["recent_requests"] = n.into();
        approval["nudge_at"] = hq_agent::governance::STREAK_NUDGE_AT.into();
    }
    item
}

/// Adds `site_approvals` to an approval item that names a site: how many times the owner
/// answered "Allow once" for that site (display only: `approve_site` works at any count). A read that fails leaves the item without the field.
fn with_site_approvals(
    conn: &rusqlite::Connection,
    mut item: NotificationItem,
) -> NotificationItem {
    let Some(approval) = item.metadata.approval.as_mut() else {
        return item;
    };
    let Some(site) = approval["site"].as_str().map(str::to_string) else {
        return item;
    };
    if let Ok(n) = hq_db::fetch_sites::allowed_once_for_host(conn, &site) {
        approval["site_approvals"] = n.into();
    }
    item
}

/// Aggregate all notifications matching filters.
pub(crate) fn aggregate_notifications(
    db: &hq_db::Database,
    state_filter: Option<&str>,
    category_filter: Option<&str>,
    limit: usize,
    include_agent: bool,
) -> Result<NotificationsResponse, anyhow::Error> {
    let mut all = scan_value_rows(db, state_filter)?;
    if !include_agent {
        all.retain(|n| n.audience == "owner");
    }
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
        .filter(|n| n.kind == NotificationKind::ActionNeeded && !n.silenced && needs_a_human(n))
        .count();
    let unread_updates_count = pending()
        .filter(|n| n.kind == NotificationKind::ValueItem && !n.read && !n.silenced)
        .count();
    let unread_count = pending().count();
    let waiting_decisions = || {
        pending()
            .filter(|n| n.kind == NotificationKind::ActionNeeded && !n.silenced && needs_a_human(n))
    };
    let needs_you_owner_only = waiting_decisions().filter(|n| n.owner_only).count();
    let needs_you_helpers = waiting_decisions().filter(|n| !n.owner_only).count();
    let expired_count = restorable_expired_count(db)?;
    all.truncate(limit);
    // Counts above never read the per-card facts, so only the rows returned pay for them.
    let all = enrich_items(db, all)?;

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
/// Allow this call and the folder it works in, for this chat (eight hours).
const ALLOW_FOLDER_ACTION: &str = "approve_folder";
/// The action on a Later offer's inbox item that brings the offer back to its chat.
const OPEN_ACTION: &str = "open";
/// What the apps write on that action's button.
const LATER_OPEN_LABEL: &str = "Bring it back";
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

/// The chat and canonical folder an "allow this folder" answer to request `val_id` would allow,
/// or the reason it cannot.
fn folder_to_allow(db: &hq_db::Database, val_id: &str) -> Result<(String, String), String> {
    use hq_agent::governance::{approval_scope, folder_scope};
    let (tool, args, key) = hq_db::owner_approvals::call_of(db, val_id)
        .map_err(|_| "the request could not be read".to_string())?
        .ok_or_else(|| "that request is not an approval".to_string())?;
    let scope =
        approval_scope(&key).ok_or_else(|| "that request does not belong to a chat".to_string())?;
    let folder = folder_scope::candidate_folder(&tool, &args, None)
        .ok_or_else(|| "this call does not name a folder that can be allowed".to_string())?;
    Ok((scope.to_string(), folder.to_string_lossy().into_owned()))
}

/// The owner approving the first-run offer to set up helpers: the setup starts on the computer
/// the offer named. Only this route, which needs the owner's sign-in, can give that yes.
fn note_helper_offer_answer(
    db: &hq_db::Database,
    val_id: &str,
    engagement: &str,
) -> anyhow::Result<()> {
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
    let Some(host) = key.as_deref().and_then(hq_tools::helper_setup::offer_host) else {
        return Ok(());
    };
    let host = hq_tools::agent_host::host(Some(host))?;
    hq_tools::helper_setup::start(db.clone(), host, "setup", &[])?;
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
        "approve" | "engage" | OPEN_ACTION | ALLOW_SITE_ACTION | ALLOW_FOLDER_ACTION => {
            Some("approved")
        }
        a if hq_tools::harness_session::sandbox_access::is_access_action(a) => Some("approved"),
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
    if !matches!(
        action,
        "approve" | "engage" | "reject" | ALLOW_SITE_ACTION | ALLOW_FOLDER_ACTION
    ) && !hq_tools::harness_session::sandbox_access::is_access_action(action)
    {
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

/// `POST /api/notifications/restore-cleared`: puts back agent-dialog notices cleared from the Inbox (the catch-up
/// card's Undo, and Restore in Done). Only an unanswered dialog notice can come back, for 30 days; a decision
/// never does. Answers how many came back.
pub(crate) async fn restore_cleared_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<RestoreManyRequest>,
) -> Response {
    if body.ids.len() > MAX_RESTORE {
        return ApiError::bad_request("too many items to restore at once").into_response();
    }
    let window = hq_db::value_items::CLEARED_DIALOG_RESTORE_DAYS * 24 * 60 * 60;
    let mut restored = 0;
    for id in &body.ids {
        let Some(val_id) = id.strip_prefix("val_") else {
            return ApiError::bad_request("unrecognized notification ID format").into_response();
        };
        match hq_db::value_items::restore_cleared_dialog(&state.db, val_id, window) {
            Ok(true) => restored += 1,
            Ok(false) => {}
            Err(e) => return ApiError::internal(e).into_response(),
        }
    }
    done(
        &state,
        serde_json::json!({ "ok": true, "restored": restored }),
    )
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
    // Reads and enriches every open item, and the client polls: keep it off the async runtime.
    let db = state.db.clone();
    let include_agent = params.include_agent.as_deref() == Some("1");
    let aggregated = tokio::task::spawn_blocking(move || {
        aggregate_notifications(
            &db,
            params.state.as_deref(),
            params.category.as_deref(),
            limit,
            include_agent,
        )
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("notifications task failed: {e}")));
    match aggregated {
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
    // `open` is only for the item of a Later offer: on anything else it would be an approval.
    if body.action == OPEN_ACTION
        && !state
            .db
            .with_conn(|c| hq_db::offers::is_later_item(c, val_id))
            .unwrap_or(false)
    {
        return ApiError::bad_request("open is only for an offer you set aside").into_response();
    }
    // A standing-access answer is checked before anything is recorded: one that cannot be applied
    // (no task, unknown starter, not an access request) is refused instead of half-answered.
    if hq_tools::harness_session::sandbox_access::is_access_action(&body.action)
        && let Err(e) = hq_tools::harness_session::sandbox_access::check_answer(
            &state.db,
            val_id,
            &body.action,
            body.starter.as_deref(),
        )
    {
        return ApiError::bad_request(e.to_string()).into_response();
    }
    // The folder is read from the call itself, never from the request, and checked before the
    // answer is recorded, so a request that names no folder is refused instead of half-answered.
    let folder_grant = if body.action == ALLOW_FOLDER_ACTION {
        match folder_to_allow(&state.db, val_id) {
            Ok(grant) => Some(grant),
            Err(refusal) => return ApiError::bad_request(refusal).into_response(),
        }
    } else {
        None
    };
    use hq_db::value_items::Answered;
    let answer = record_action(&state.db, val_id, engagement);
    // A repeat of an answer already given must not count as another approval.
    let first_answer = matches!(answer, Ok(Answered::Recorded));
    // The owner can upgrade an answer already given (Allow, then Allow for the repository), and a
    // failed rule write must be retryable, so standing access applies on a repeat too.
    let answered = matches!(answer, Ok(Answered::Recorded | Answered::AlreadySo));
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
    // Standing access for the task or repository, and what it added, so the owner is told when the
    // rule did not land instead of believing it did.
    let mut rules_added: Option<usize> = None;
    if answered && hq_tools::harness_session::sandbox_access::is_access_action(&body.action) {
        match hq_tools::harness_session::sandbox_access::apply_answer(
            &state.db,
            val_id,
            &body.action,
            body.starter.as_deref(),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(n) => rules_added = Some(n),
            Err(e) => {
                warn!(id = %id, error = %e, "recording standing sandbox access failed");
                return ApiError::internal(format!(
                    "Your answer was recorded, but the standing access was not saved: {e}. Try again."
                ))
                .into_response();
            }
        }
    }
    if first_answer {
        let whole_site = body.action == ALLOW_SITE_ACTION;
        if let Err(e) = note_site_answer(&state.db, val_id, engagement, whole_site) {
            warn!(id = %id, error = %e, "recording a site's approval failed");
        }
        if let Some((scope, root)) = &folder_grant
            && let Err(e) = hq_db::owner_folder_grants::grant(
                &state.db,
                scope,
                root,
                hq_db::owner_folder_grants::DEFAULT_TTL_SECS,
            )
        {
            warn!(id = %id, error = %e, "recording a folder allow failed");
        }
        if let Err(e) = note_helper_offer_answer(&state.db, val_id, engagement) {
            warn!(id = %id, error = %e, "starting the offered helper setup failed");
        }
        if let Err(e) = note_first_task_answer(&state.db, val_id, engagement) {
            warn!(id = %id, error = %e, "recording the first coding task's yes failed");
        }
        // The notes check's own items: an approval fixes the links, or allows doing so.
        let (vault, db) = (state.vault_path.clone(), state.db.clone());
        let (answer_id, approved) = (val_id.to_string(), engagement == ENGAGEMENT_APPROVED);
        let answered = tokio::task::spawn_blocking(move || {
            hq_tools::vault_health::answer_inbox(
                &vault,
                &std::sync::Arc::new(db),
                &answer_id,
                approved,
            )
        })
        .await;
        match answered {
            Ok(Ok(())) => state.announce_change("vault-health"),
            Ok(Err(e)) => warn!(id = %id, error = %e, "fixing the notes from the Inbox failed"),
            Err(e) => warn!(id = %id, error = %e, "fixing the notes from the Inbox did not finish"),
        }
    }
    if first_answer && body.action == OPEN_ACTION {
        // Bring the offer back now; the daemon pass would find nothing left to do.
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = state.db.with_conn(|c| hq_db::offers::deliver_later(c, now)) {
            warn!(id = %id, error = %e, "bringing a Later offer back failed; the daemon will retry");
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
        message: match rules_added {
            Some(n) => format!("Value item {engagement}; {n} standing access rule(s) added"),
            None => format!("Value item {engagement}"),
        },
    })
    .into_response()
}

/// Tells every open tab that the inbox changed, with the counts its badge shows.
fn announce_inbox(state: &WsState) {
    state.announce_change(crate::CHANGE_INBOX);
    if let Ok(res) = aggregate_notifications(&state.db, Some("pending"), None, 50, false) {
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
mod tests;
