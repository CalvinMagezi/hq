//! `GET` and `PUT /api/prefs`: the few choices the owner makes in the web app that HQ's own work reads. Only the keys in
//! `hq_db::owner_prefs::KNOWN` exist, each with its own values; nothing else can be written through here.

use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse, response::Response};
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;
use hq_db::owner_prefs::{CHAT_AUTO_TITLE, HELPERS_ASK_LEVEL};

/// The effective value of every pref: what the owner chose, else what HQ does without a choice.
pub(crate) fn effective(state: &WsState) -> Value {
    let auto_title = hq_db::owner_prefs::auto_title_on(&state.db);
    let ask = hq_db::owner_prefs::get(&state.db, HELPERS_ASK_LEVEL)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            // Without a choice the config file's rule stands, which is "nothing" unless it says otherwise.
            hq_core::config::HqConfig::load()
                .map(|c| c.inbox.fleet.helper_policy.as_str().to_string())
                .unwrap_or_else(|_| "nothing".to_string())
        });
    json!({ CHAT_AUTO_TITLE: auto_title, HELPERS_ASK_LEVEL: ask })
}

pub(crate) async fn prefs_get_handler(State(state): State<Arc<WsState>>) -> Response {
    Json(effective(&state)).into_response()
}

/// `{ "key": "...", "value": "..." }`. A bool pref takes a JSON bool or the words `true` and `false`.
pub(crate) async fn prefs_put_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let key = body.get("key").and_then(Value::as_str).unwrap_or("");
    let value = match body.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    };
    if !hq_db::owner_prefs::KNOWN.contains(&key) {
        return ApiError::bad_request("that is not a choice HQ keeps").into_response();
    }
    if !hq_db::owner_prefs::valid(key, &value) {
        return ApiError::bad_request(format!("'{value}' is not a choice for {key}"))
            .into_response();
    }
    match hq_db::owner_prefs::set(&state.db, key, &value) {
        Ok(()) => Json(effective(&state)).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    async fn body(res: Response) -> Value {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn a_choice_is_stored_and_read_back_and_nothing_else_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let first = body(prefs_get_handler(State(state.clone())).await).await;
        assert_eq!(
            first[CHAT_AUTO_TITLE], true,
            "chat titles are on by default"
        );

        let off = prefs_put_handler(
            State(state.clone()),
            Json(json!({ "key": CHAT_AUTO_TITLE, "value": false })),
        )
        .await;
        assert_eq!(off.status(), StatusCode::OK);
        assert_eq!(body(off).await[CHAT_AUTO_TITLE], false);
        let ask = prefs_put_handler(
            State(state.clone()),
            Json(json!({ "key": HELPERS_ASK_LEVEL, "value": "loses_work" })),
        )
        .await;
        assert_eq!(body(ask).await[HELPERS_ASK_LEVEL], "loses_work");

        for bad in [
            json!({ "key": "other.thing", "value": "x" }),
            json!({ "key": HELPERS_ASK_LEVEL, "value": "always" }),
            json!({ "key": CHAT_AUTO_TITLE }),
        ] {
            let res = prefs_put_handler(State(state.clone()), Json(bad)).await;
            assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        }
    }
}
