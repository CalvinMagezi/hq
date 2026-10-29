//! Answering a session's dialog from a notification (`docs/plans/remote-driven-teams.md`,
//! "Design for the answer buttons").
//!
//! An inbox item raised for a blocked session carries the dialog it is about
//! (`hq_db::dialog_alerts`). A button answers that dialog and no other: the approve button picks
//! the one option the server chose when the alert was raised (a plain yes, once), and decline
//! dismisses it. The phone and the web app call the route behind the web token; a web push button
//! calls [`PUSH_ANSWER_PATH`] with a token the push carried, signed with the install's approval
//! key for that item, dialog number and choice, valid for [`TOKEN_TTL_SECS`] and good for one
//! answer.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::{IntoResponse, Response},
};
use hq_db::dialog_alerts::{self, DialogAlert};
use hq_db::harness_sessions_registry as registry;
use hq_tools::agent_host::AgentHostError;
use hq_tools::harness_session::answer::{self as session_answer, Answerer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

/// The one notifications path that takes no web token: a web push button posts here with the
/// token its push carried.
pub(crate) const PUSH_ANSWER_PATH: &str = "/api/notifications/push-answer";

/// How long a push's answer tokens stay good.
pub(crate) const TOKEN_TTL_SECS: i64 = 15 * 60;

const ALREADY_ANSWERED: &str = "That question was already answered, or the agent has moved on.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Choice {
    Approve,
    Decline,
}

impl Choice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Choice::Approve => "approve",
            Choice::Decline => "decline",
        }
    }
}

/// A token for one answer to the dialog behind `alert`: `<expires>.<hmac>`.
pub(crate) fn token(key: &[u8], alert: &DialogAlert, choice: Choice, expires: i64) -> String {
    let message = hq_core::approval_key::dialog_answer_message(
        &alert.item_id,
        alert.pending_seq,
        choice.as_str(),
        expires,
    );
    format!(
        "{expires}.{}",
        hq_core::approval_key::hmac_hex(key, &message)
    )
}

/// Whether `token` was minted for this item, dialog number and choice, and has not expired.
pub(crate) fn token_valid(
    key: &[u8],
    alert: &DialogAlert,
    choice: Choice,
    token: &str,
    now: i64,
) -> bool {
    let Some((expires, mac)) = token.split_once('.') else {
        return false;
    };
    let Ok(expires) = expires.parse::<i64>() else {
        return false;
    };
    if now > expires || expires > now + TOKEN_TTL_SECS {
        return false;
    }
    let message = hq_core::approval_key::dialog_answer_message(
        &alert.item_id,
        alert.pending_seq,
        choice.as_str(),
        expires,
    );
    hq_core::approval_key::verify_hex(key, &message, mac)
}

/// The dialog as a notification shows it: what it asks, the buttons it may have, whether it is
/// already answered. `approve` is null when no option may be picked from a button.
pub(crate) fn dialog_view(alert: &DialogAlert) -> Value {
    json!({
        "session_id": alert.session_id,
        "seq": alert.pending_seq,
        "kind": alert.kind,
        "question": alert.question,
        "command": alert.command,
        "options": alert.options,
        "approve": alert.approve_option.map(|id| json!({ "id": id, "label": alert.approve_label() })),
        "answered": alert.answered,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnswerBody {
    choice: Choice,
}

/// `POST /api/notifications/{id}/answer`, behind the web token: the phone and the web app.
pub(crate) async fn answer_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<AnswerBody>,
) -> Response {
    answer(&state, &id, body.choice, None).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct PushAnswerBody {
    id: String,
    choice: Choice,
    token: String,
}

/// [`PUSH_ANSWER_PATH`]: a web push button, with no web token, only the answer token its push
/// carried.
pub(crate) async fn push_answer_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<PushAnswerBody>,
) -> Response {
    answer(&state, &body.id, body.choice, Some(body.token)).await
}

async fn answer(state: &Arc<WsState>, id: &str, choice: Choice, token: Option<String>) -> Response {
    let Some(item_id) = id.strip_prefix("val_").map(str::to_string) else {
        return ApiError::bad_request(format!("unrecognized notification ID format '{id}'"))
            .into_response();
    };
    let db = Arc::new(state.db.clone());
    let result =
        tokio::task::spawn_blocking(move || answer_blocking(&db, &item_id, choice, token)).await;
    let report = match result {
        Ok(Ok(report)) => report,
        Ok(Err(e)) => return e.into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    state.broadcast(
        &json!({
            "type": "notification:resolved",
            "id": id,
            "action": choice.as_str(),
            "new_state": if choice == Choice::Approve { "approved" } else { "dismissed" },
        })
        .to_string(),
    );
    Json(report).into_response()
}

fn answer_blocking(
    db: &Arc<hq_db::Database>,
    item_id: &str,
    choice: Choice,
    token: Option<String>,
) -> Result<Value, ApiError> {
    let expired = || ApiError::Forbidden("this button has expired; open HQ to answer".into());
    let Some(alert) = dialog_alerts::get(db, item_id).map_err(ApiError::internal)? else {
        // A caller with no web token learns nothing about which items exist.
        return Err(match token {
            Some(_) => expired(),
            None => ApiError::NotFound("this notice has no question to answer".into()),
        });
    };
    if let Some(token) = token {
        let key = hq_core::approval_key::load_or_create_key().map_err(ApiError::internal)?;
        let now = chrono::Utc::now().timestamp();
        if !token_valid(&key, &alert, choice, &token, now) {
            return Err(expired());
        }
    }
    if alert.answered {
        return Err(ApiError::Conflict(ALREADY_ANSWERED.into()));
    }
    let picked = match choice {
        Choice::Approve => Some(alert.approve_option.ok_or_else(|| {
            ApiError::Conflict(
                "this question has no yes-once answer; open the session to answer it".into(),
            )
        })?),
        Choice::Decline => None,
    };
    if !dialog_alerts::claim(db, item_id, choice.as_str()).map_err(ApiError::internal)? {
        return Err(ApiError::Conflict(ALREADY_ANSWERED.into()));
    }
    let answered = session_answer::answer(
        db,
        &alert.session_id,
        alert.pending_seq,
        match picked {
            Some(id) => session_answer::Choice::Option(id),
            None => session_answer::Choice::Decline(None),
        },
        Answerer::Owner,
        None,
    );
    let report = match answered {
        Ok(report) => report,
        Err(e) if moved_on(&e) => {
            // The dialog is gone for good, so the claim stays: nothing can answer it now.
            resolve(db, item_id, Choice::Decline);
            return Err(ApiError::Conflict(ALREADY_ANSWERED.into()));
        }
        Err(e) => {
            // Nothing was answered; give the claim back so the button can be tried again.
            let _ = dialog_alerts::release(db, item_id);
            tracing::warn!(item = %item_id, error = %e, "dialog answer: could not reach the session");
            return Err(match e.downcast_ref::<AgentHostError>() {
                Some(err) if err.is_unreachable() => {
                    ApiError::Unavailable("the agent's computer cannot be reached right now".into())
                }
                _ => ApiError::Internal("the answer could not be sent".into()),
            });
        }
    };
    resolve(db, item_id, choice);
    let detail = format!(
        "{} dialog {} from a notification",
        if choice == Choice::Approve {
            "approved"
        } else {
            "declined"
        },
        alert.pending_seq
    );
    let audited = db.with_conn(|c| {
        registry::record_event(
            c,
            &alert.session_id,
            registry::EVENT_SENT,
            registry::ACTOR_USER,
            Some(&detail),
        )
    });
    if let Err(e) = audited {
        tracing::warn!(session = %alert.session_id, error = %e, "dialog answer: delivered but not audited");
    }
    Ok(json!({
        "answered": choice.as_str(),
        "label": picked.and_then(|_| alert.approve_label()),
        "registered": report["registered"],
        "session_id": alert.session_id,
    }))
}

/// The answer found the dialog gone: answered elsewhere, replaced, or the session ended.
fn moved_on(e: &anyhow::Error) -> bool {
    if let Some(AgentHostError::Api { code, .. }) = e.downcast_ref::<AgentHostError>() {
        return matches!(
            code.as_str(),
            "stale_dialog" | "no_dialog" | "agent_not_found" | "agent_exited"
        );
    }
    let msg = e.to_string();
    [
        "at a different dialog now",
        "not waiting at a dialog",
        "is not running",
        "no harness session",
    ]
    .iter()
    .any(|m| msg.contains(m))
}

/// Takes the item off every device's list.
fn resolve(db: &hq_db::Database, item_id: &str, choice: Choice) {
    let (engagement, state) = crate::notifications_api::action_outcome(match choice {
        Choice::Approve => "approve",
        Choice::Decline => "dismiss",
    })
    .unwrap_or(("dismissed", "dismissed"));
    if let Err(e) = crate::notifications_api::record_action(db, item_id, engagement, state) {
        tracing::warn!(item = %item_id, error = %e, "dialog answer: could not resolve the inbox item");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::dialog_alerts::DialogOption;

    fn alert(answered: bool) -> DialogAlert {
        DialogAlert {
            item_id: "item-1".into(),
            session_id: "hs-1".into(),
            pending_seq: 7,
            kind: "path".into(),
            question: Some("Do you want to allow this?".into()),
            command: Some("/tmp".into()),
            options: vec![
                DialogOption {
                    id: 1,
                    label: "Yes".into(),
                    remember: false,
                    typed: false,
                },
                DialogOption {
                    id: 2,
                    label: "Yes, and remember this path".into(),
                    remember: true,
                    typed: false,
                },
            ],
            approve_option: Some(1),
            answered,
        }
    }

    #[test]
    fn a_token_answers_one_dialog_one_way_until_it_expires() {
        let key = [7u8; 32];
        let a = alert(false);
        let now = 1_800_000_000;
        let t = token(&key, &a, Choice::Approve, now + TOKEN_TTL_SECS);
        assert!(token_valid(&key, &a, Choice::Approve, &t, now));
        assert!(!token_valid(&key, &a, Choice::Decline, &t, now), "one way");
        let mut next_dialog = a.clone();
        next_dialog.pending_seq = 8;
        assert!(
            !token_valid(&key, &next_dialog, Choice::Approve, &t, now),
            "one dialog"
        );
        let mut other_item = a.clone();
        other_item.item_id = "item-2".into();
        assert!(!token_valid(&key, &other_item, Choice::Approve, &t, now));
        assert!(
            !token_valid(&key, &a, Choice::Approve, &t, now + TOKEN_TTL_SECS + 1),
            "expired"
        );
        assert!(!token_valid(&[8u8; 32], &a, Choice::Approve, &t, now));
        // An expiry pushed out past the limit is refused even with a valid MAC for it.
        let far = token(&key, &a, Choice::Approve, now + 10 * TOKEN_TTL_SECS);
        assert!(!token_valid(&key, &a, Choice::Approve, &far, now));
        for junk in ["", "nodot", "x.y", &format!("{}.", now + 60)] {
            assert!(
                !token_valid(&key, &a, Choice::Approve, junk, now),
                "{junk:?}"
            );
        }
    }

    #[test]
    fn the_view_offers_only_the_yes_once_option() {
        let v = dialog_view(&alert(false));
        assert_eq!(v["approve"]["id"], 1);
        assert_eq!(v["approve"]["label"], "Yes");
        assert_eq!(v["seq"], 7);
        assert_eq!(v["answered"], false);
        let mut none = alert(false);
        none.approve_option = None;
        assert!(dialog_view(&none)["approve"].is_null());
    }

    fn seeded(answered: bool) -> Arc<hq_db::Database> {
        let db = Arc::new(hq_db::Database::open_memory().unwrap());
        let a = alert(false);
        dialog_alerts::record(
            &db,
            &dialog_alerts::NewDialogAlert {
                item_id: &a.item_id,
                session_id: &a.session_id,
                pending_seq: a.pending_seq,
                kind: &a.kind,
                question: a.question.as_deref(),
                command: a.command.as_deref(),
                options: &a.options,
            },
        )
        .unwrap();
        if answered {
            dialog_alerts::claim(&db, "item-1", "approve").unwrap();
        }
        db
    }

    #[test]
    fn an_answered_or_unknown_dialog_is_refused_before_anything_is_sent() {
        let db = seeded(true);
        let err = answer_blocking(&db, "item-1", Choice::Approve, None).unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
        let err = answer_blocking(&db, "nope", Choice::Decline, None).unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)), "{err:?}");
        // Without the web token, an unknown item looks the same as a bad token.
        let err = answer_blocking(&db, "nope", Choice::Decline, Some("1.ab".into())).unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(_)), "{err:?}");
    }

    #[test]
    fn a_session_that_is_gone_keeps_the_claim_and_reads_as_already_answered() {
        // No harness session row for hs-1: the dialog cannot be answered, and never will be.
        let db = seeded(false);
        let err = answer_blocking(&db, "item-1", Choice::Decline, None).unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
        assert!(dialog_alerts::get(&db, "item-1").unwrap().unwrap().answered);
    }
}
