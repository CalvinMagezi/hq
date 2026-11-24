//! `POST /api/finance/import/{action_id}`: where the phone hands over the messages a
//! `finance_import_messages` call approved. The messages go to the ledger and nowhere else: the
//! route settles the action with counts, so the model never sees a message, and the body is not
//! logged. The action id is random, single use and tied to an approved, acknowledged action, so it
//! is the only credential the route takes. It sits behind the same sign-in and origin guard as
//! every other `/api` route, which an MCP key does not pass.

use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use hq_finance::store::{Message, Receipt, import_messages};
use hq_tools::device_actions::{Reject, finalize_for};
use hq_tools::finance::IMPORT_ACTION_NAME;
use serde_json::{Value, json};

use crate::WsState;

pub(crate) const IMPORT_PATH: &str = "/api/finance/import/{action_id}";

/// Two thousand messages of a few hundred characters each.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_MESSAGES: usize = 2_000;

pub(crate) fn routes(router: axum::Router<Arc<WsState>>) -> axum::Router<Arc<WsState>> {
    router.route(
        IMPORT_PATH,
        post(import_handler).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
    )
}

#[derive(serde::Deserialize)]
struct Upload {
    messages: Vec<Message>,
}

fn refusal(status: StatusCode, reason: &str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

fn rejected(why: Reject) -> Response {
    match why {
        Reject::Unknown => refusal(StatusCode::NOT_FOUND, "unknown_action"),
        Reject::Expired => refusal(StatusCode::GONE, "expired"),
        Reject::Resolved => refusal(StatusCode::CONFLICT, "already_resolved"),
    }
}

/// The senders HQ told the phone to read, taken from the action's own payload so a phone cannot
/// widen them. Anything the payload does not name is dropped before parsing.
fn allowed_senders(payload: &Value) -> Vec<String> {
    payload
        .get("senders")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn settle_body(receipt: &Receipt) -> Value {
    json!({
        "v": 1,
        "outcome": hq_tools::finance::outcome::IMPORTED,
        "seen": receipt.seen,
        "ignored": receipt.ignored,
        "transactions": receipt.transactions,
        "added": receipt.added,
        "waiting_for_review": receipt.waiting_for_review,
    })
}

async fn import_handler(
    State(state): State<Arc<WsState>>,
    Path(action_id): Path<String>,
    Json(upload): Json<Upload>,
) -> Response {
    let payload = match state
        .device_actions
        .claim_upload(&action_id, IMPORT_ACTION_NAME)
    {
        Ok(p) => p,
        Err(why) => return rejected(why),
    };
    if upload.messages.len() > MAX_MESSAGES {
        return refusal(StatusCode::PAYLOAD_TOO_LARGE, "too_many_messages");
    }
    let senders = allowed_senders(&payload);
    let vault = state.vault_path.clone();
    let messages = upload.messages;
    let result = tokio::task::spawn_blocking(move || {
        let sender_refs: Vec<&str> = senders.iter().map(String::as_str).collect();
        import_messages(&vault, &messages, &sender_refs)
    })
    .await;
    let body = match result {
        Ok(Ok(receipt)) => settle_body(&receipt),
        // The reason is a fixed word: an I/O error can quote a path.
        _ => {
            json!({"v": 1, "outcome": hq_tools::finance::outcome::READ_FAILED, "reason": "ledger_not_written"})
        }
    };
    let finalize = |b: &Value, p: &Value| finalize_for(IMPORT_ACTION_NAME, b, p);
    if let Err(why) = state.device_actions.settle(&action_id, &body, finalize) {
        return rejected(why);
    }
    // Counts only: they are the same numbers the model is told.
    tracing::info!(%action_id, phase = "finance_import", "device action");
    if body["outcome"] == hq_tools::finance::outcome::IMPORTED {
        Json(body).into_response()
    } else {
        refusal(StatusCode::INTERNAL_SERVER_ERROR, "ledger_not_written")
    }
}

#[cfg(test)]
#[path = "finance_api_tests.rs"]
mod tests;
