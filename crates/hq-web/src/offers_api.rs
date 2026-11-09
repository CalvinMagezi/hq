//! Offers under a reply, on the web and phone chats (`docs/plans/simple-hq.md`, phase 2): which
//! one a finished owner turn gets, noting that it was shown, and `POST /api/offers/answer` for
//! Yes and Not now. HQ chooses on the device, from deterministic detectors and then, for a task
//! the turn worked on, the on-device embedder; no language model decides an offer or is told about
//! one. What was shown and answered is adoption data and stays in `offer_log`
//! (`docs/security/ADOPTION_DATA.md`).

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_tools::capabilities::next_step;
use hq_tools::capabilities::offers::{TurnInput, TurnOffer, for_turn, ready_detectors};
use hq_tools::capabilities::{Install, Offer, catalog, resolve, used_tools};
use hq_tools::registry::ToolRegistry;
use hq_tools::vault::configured_query_embedder;
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

fn offers_here(db: &hq_db::Database, config: &HqConfig, ctx: &OfferContext) -> Vec<Offer> {
    let registry = ctx.registry.clone();
    let has_tool = |name: &str| registry.as_ref().is_none_or(|r| r.get(name).is_some());
    resolve(
        catalog(),
        &Install::detect(config),
        &has_tool,
        &used_tools(db),
    )
}

/// The offer for a reply, if one fits: ready here, not muted, not too soon after the last one,
/// and not something the turn already did with `turn_tools`. A failure means no offer.
fn choose_by_rule(
    db: &hq_db::Database,
    config: &HqConfig,
    ctx: &OfferContext,
    turn_tools: &[String],
) -> Option<TurnOffer> {
    let offers = offers_here(db, config, ctx);
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

/// A next step for the task this turn worked on, when this instance embeds on the device.
async fn choose_next_step(
    db: hq_db::Database,
    config: Arc<HqConfig>,
    ctx: OfferContext,
    turn_tools: Vec<String>,
) -> Option<TurnOffer> {
    let weigh = (db.clone(), turn_tools.clone(), ctx.asked_at);
    let (embed, task) = tokio::task::spawn_blocking(move || {
        let (embed, _) = configured_query_embedder()?;
        let now = chrono::Utc::now().timestamp();
        let task = weigh
            .0
            .with_conn(|c| next_step::task_to_weigh(c, &weigh.1, weigh.2, now))
            .inspect_err(|e| tracing::warn!("offers: could not read the turn's task: {e}"))
            .ok()??;
        Some((embed, task))
    })
    .await
    .ok()??;
    let capability = next_step::nearest_capability(&embed, &next_step::query_text(&task)).await?;
    tokio::task::spawn_blocking(move || {
        let ready: HashSet<String> = offers_here(&db, &config, &ctx)
            .into_iter()
            .filter(Offer::is_ready)
            .map(|o| o.id)
            .collect();
        let now = chrono::Utc::now().timestamp();
        db.with_conn(|c| next_step::offer_for(c, capability, &task.title, &ready, &turn_tools, now))
            .inspect_err(|e| tracing::warn!("offers: could not build a next step: {e}"))
            .ok()
            .flatten()
    })
    .await
    .ok()?
}

/// The one offer for a reply: a rule's if any fits, otherwise a next step.
pub(crate) async fn choose(
    db: hq_db::Database,
    config: Arc<HqConfig>,
    ctx: OfferContext,
    turn_tools: Vec<String>,
) -> Option<TurnOffer> {
    let rule = {
        let (db, config, ctx, tools) =
            (db.clone(), config.clone(), ctx.clone(), turn_tools.clone());
        tokio::task::spawn_blocking(move || choose_by_rule(&db, &config, &ctx, &tools))
            .await
            .ok()
            .flatten()
    };
    match rule {
        Some(offer) => Some(offer),
        None => choose_next_step(db, config, ctx, turn_tools).await,
    }
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
