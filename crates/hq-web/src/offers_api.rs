//! Offers under a reply, on the web and phone chats (`docs/plans/simple-hq.md`, phase 2): which
//! one a finished owner turn gets, noting that it was shown, and `POST /api/offers/answer` for
//! Yes and Not now. HQ chooses on the device from deterministic detectors; the model never
//! decides an offer and is never told about one. What was shown and answered is adoption data and
//! stays in `offer_log` (`docs/security/ADOPTION_DATA.md`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_tools::capabilities::offers::{TurnInput, TurnOffer, for_turn, ready_detectors};
use hq_tools::capabilities::{Install, catalog, resolve, used_tools};
use hq_tools::registry::ToolRegistry;
use serde::Deserialize;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

/// What an owner turn needs to pick an offer once its reply is in.
#[derive(Clone)]
pub(crate) struct OfferContext {
    /// Taken just before the person's message was saved, so history before it can be told apart.
    pub asked_at: i64,
    /// The message as saved, with its attachment marker.
    pub text: String,
    pub registry: Option<Arc<ToolRegistry>>,
}

/// The offer for a reply, if one fits: ready here, not muted, not too soon after the last one,
/// and not something the turn already did with `turn_tools`. A failure means no offer.
pub(crate) fn choose(
    db: &hq_db::Database,
    config: &HqConfig,
    ctx: &OfferContext,
    turn_tools: &[String],
) -> Option<TurnOffer> {
    let registry = ctx.registry.clone();
    let has_tool = |name: &str| registry.as_ref().is_none_or(|r| r.get(name).is_some());
    let offers = resolve(
        catalog(),
        &Install::detect(config),
        &has_tool,
        &used_tools(db),
    );
    let input = TurnInput {
        text: &ctx.text,
        asked_at: ctx.asked_at,
        now: chrono::Utc::now().timestamp(),
        turn_tools,
        ready: ready_detectors(&offers),
        today: chrono::Local::now().date_naive(),
    };
    db.with_conn(|c| for_turn(c, &input))
        .inspect_err(|e| tracing::warn!("offers: could not choose an offer: {e}"))
        .ok()
        .flatten()
}

/// Notes that the reply `message_id` carries an offer of `kind`.
pub(crate) fn record_shown(db: &hq_db::Database, message_id: &str, kind: &str) {
    let at = chrono::Utc::now().timestamp();
    if let Err(e) = db.with_conn(|c| hq_db::offers::record_shown(c, message_id, kind, at)) {
        tracing::warn!("offers: could not note a shown offer: {e}");
    }
}

#[derive(Deserialize)]
pub(crate) struct AnswerBody {
    message_id: String,
    answer: String,
}

/// `POST /api/offers/answer`: Yes or Not now to the offer under one reply. The offer leaves the
/// reply's saved copy and the answer is kept only in `offer_log`. 404 when that reply has no
/// open offer.
pub(crate) async fn answer_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<AnswerBody>,
) -> Result<impl IntoResponse, ApiError> {
    let Some(answer) = hq_db::offers::Answer::parse(&body.answer) else {
        return Err(ApiError::bad_request("answer is yes or not_now"));
    };
    let db = state.db.clone();
    let at = chrono::Utc::now().timestamp();
    let answered = tokio::task::spawn_blocking(move || {
        db.with_conn(|c| hq_db::offers::answer(c, &body.message_id, answer, at))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    match answered {
        Some(_) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
        None => Err(ApiError::not_found()),
    }
}
