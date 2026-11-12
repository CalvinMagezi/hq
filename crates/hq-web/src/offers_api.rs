//! Offers under a reply, on the web and phone chats (`docs/plans/simple-hq.md`, phase 2): which
//! one a finished owner turn gets, noting that it was shown, and `POST /api/offers/answer` for
//! Yes and Not now. HQ chooses on the device, from deterministic detectors and then, for a task
//! the turn worked on, the on-device embedder; no language model decides an offer or is told about
//! one. What was shown and answered is adoption data and stays in `offer_log` and
//! `offer_type_memory` (`docs/security/ADOPTION_DATA.md`). Besides Yes and Not now the routes take
//! Later, Never show and Fewer like this, and give Settings the counts and a Clear
//! (`docs/plans/offers-later.md`).

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_core::types::{SCHEDULED_OFFER_SOURCE, ValueItem, ValueKind};
use hq_db::offers::{Answer, LaterOutcome, MAX_SCHEDULE_SECS};
use hq_tools::capabilities::next_step;
use hq_tools::capabilities::offers::{
    TurnInput, TurnOffer, TurnState, for_turn, may_offer, offer_says, ready_detectors,
};
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
    /// The turn is part of a voice conversation, which never carries an offer.
    pub voice: bool,
}

fn offers_here(db: &hq_db::Database, install: &Install, ctx: &OfferContext) -> Vec<Offer> {
    let registry = ctx.registry.clone();
    let has_tool = |name: &str| registry.as_ref().is_none_or(|r| r.get(name).is_some());
    resolve(catalog(), install, &has_tool, &used_tools(db))
}

/// The offer for a reply, if one fits: ready here, not muted, not too soon after the last one,
/// and not something the turn already did with `turn_tools`. A failure means no offer.
fn choose_by_rule(
    db: &hq_db::Database,
    install: &Install,
    ctx: &OfferContext,
    turn_tools: &[String],
) -> Option<TurnOffer> {
    let offers = offers_here(db, install, ctx);
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
        let ready: HashSet<String> = offers_here(&db, &Install::detect(&config), &ctx)
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

/// The rule-based offer for a reply on `install`. `None` when the cooldowns (voice, an approval
/// waiting, an error, the hourly cap) allow no offer at all; `Some(None)` when they allow one but
/// no rule fits.
async fn rule_offer(
    db: hq_db::Database,
    install: Install,
    ctx: OfferContext,
    turn_tools: Vec<String>,
    state: TurnState,
) -> Option<Option<TurnOffer>> {
    tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();
        let calm = db
            .with_conn(|c| may_offer(c, state, now))
            .inspect_err(|e| tracing::warn!("offers: could not check the cooldowns: {e}"))
            .unwrap_or(false);
        calm.then(|| choose_by_rule(&db, &install, &ctx, &turn_tools))
    })
    .await
    .ok()
    .flatten()
}

/// The one offer for a reply: a rule's if any fits, otherwise a next step.
pub(crate) async fn choose(
    db: hq_db::Database,
    config: Arc<HqConfig>,
    ctx: OfferContext,
    turn_tools: Vec<String>,
    state: TurnState,
) -> Option<TurnOffer> {
    let install = Install::detect(&config);
    let rule = rule_offer(db.clone(), install, ctx.clone(), turn_tools.clone(), state).await?;
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

/// The answer that brings an offer back at a time the person picked.
const SCHEDULED: &str = "scheduled";

#[derive(Deserialize)]
pub(crate) struct AnswerBody {
    message_id: String,
    answer: String,
    /// When a scheduled offer should come back, in unix seconds.
    at: Option<i64>,
}

/// The time a `scheduled` answer asks for, which must be ahead of now and not too far. Other
/// answers have none.
fn schedule_time(answer: &str, at: Option<i64>, now: i64) -> Result<Option<i64>, ApiError> {
    if answer != SCHEDULED {
        return Ok(None);
    }
    match at {
        Some(due) if due > now && due - now <= MAX_SCHEDULE_SECS => Ok(Some(due)),
        _ => Err(ApiError::bad_request(
            "scheduled needs a time ahead of now and within 90 days",
        )),
    }
}

/// The sentence of the quiet inbox item a Later offer leaves.
fn later_title(kind: &str) -> String {
    let says = offer_says(kind);
    let mut chars = says.chars();
    let lowered: String = chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default();
    format!("Still want to {lowered}?")
}

/// The one inbox item a Later offer leaves. It is a plain update for the owner's own devices
/// (`SCHEDULED_OFFER_SOURCE` is device-only: never pushed), and it is the only resurfacing.
fn later_item(item_id: &str, kind: &str, thread_id: &str) -> ValueItem {
    let mut item = ValueItem::new(
        SCHEDULED_OFFER_SOURCE,
        ValueKind::Fyi,
        later_title(kind),
        "Open this to bring it back to your chat.".to_string(),
    )
    .with_dedup_key(format!("{}{item_id}", hq_db::offers::LATER_ITEM_PREFIX))
    // The chat the offer comes back to, so a client opens that chat and not whichever is open.
    .with_link(format!("/chat?thread={thread_id}"));
    item.id = item_id.to_string();
    item
}

/// `POST /api/offers/answer`: Yes, Not now, Later or a scheduled time for the offer under one
/// reply. The offer leaves the reply's saved copy and the answer is kept only on the device. 404
/// when that reply has no open offer.
pub(crate) async fn answer_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<AnswerBody>,
) -> Result<impl IntoResponse, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let due = schedule_time(&body.answer, body.at, now)?;
    let answer = match Answer::parse(&body.answer) {
        Some(answer) => answer,
        None if due.is_some() => Answer::Yes,
        None => {
            return Err(ApiError::bad_request(
                "answer is yes, not_now, later or scheduled",
            ));
        }
    };
    let item_id = uuid::Uuid::new_v4().to_string();
    let db = state.db.clone();
    let answered = tokio::task::spawn_blocking(move || {
        db.with_conn(|c| match (due, answer) {
            (Some(due_at), _) => Ok(hq_db::offers::schedule(c, &body.message_id, due_at, now)?
                .map_or(LaterOutcome::NoOffer, LaterOutcome::Parked)),
            // The parking row and its inbox item are written together, or not at all.
            (None, Answer::Later) => hq_db::offers::later(
                c,
                &body.message_id,
                |kind, thread| later_item(&item_id, kind, thread),
                now,
            ),
            (None, _) => Ok(hq_db::offers::answer(c, &body.message_id, answer, now)?
                .map_or(LaterOutcome::NoOffer, LaterOutcome::Parked)),
        })
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    match answered {
        LaterOutcome::NoOffer => Err(ApiError::not_found()),
        LaterOutcome::AlreadyUsed => Err(ApiError::Conflict(
            "That offer already came back from Later once.".into(),
        )),
        LaterOutcome::Parked(_) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
    }
}

#[derive(Deserialize)]
pub(crate) struct MessageBody {
    message_id: String,
}

/// `POST /api/offers/later/open`: the person answers a Later offer from the chat instead of the
/// inbox. The offer returns to the chat now and its inbox item is settled. 404 when nothing is
/// parked for that reply.
pub(crate) async fn later_open_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<MessageBody>,
) -> Result<impl IntoResponse, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let db = state.db.clone();
    let back = tokio::task::spawn_blocking(move || {
        db.with_conn(|c| hq_db::offers::bring_back(c, &body.message_id, now))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    match back {
        Some(_) => Ok(Json(json!({ "ok": true }))),
        None => Err(ApiError::not_found()),
    }
}

#[derive(Deserialize)]
pub(crate) struct TypeBody {
    /// An offer kind (`dated_todo`) or, for a pill, a capability id (`find-notes`).
    r#type: String,
}

/// Whether `t` names a kind of offer or a capability HQ has.
fn known_type(t: &str) -> bool {
    hq_db::offers::is_mapped(t) || catalog().iter().any(|c| c.id == t)
}

async fn remember(
    state: &Arc<WsState>,
    body: TypeBody,
    set: fn(&rusqlite::Connection, &str, i64) -> anyhow::Result<()>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !known_type(&body.r#type) {
        return Err(ApiError::bad_request("type is not a kind of offer"));
    }
    let (db, now) = (state.db.clone(), chrono::Utc::now().timestamp());
    tokio::task::spawn_blocking(move || db.with_conn(|c| set(c, &body.r#type, now)))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/offers/never { type }`: never show this kind of offer or pill again, until a Clear.
pub(crate) async fn never_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<TypeBody>,
) -> Result<impl IntoResponse, ApiError> {
    remember(&state, body, hq_db::offers::set_never).await
}

/// `POST /api/offers/fewer { type }`: a "Not now" on this type hides it twice as long for 30 days.
pub(crate) async fn fewer_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<TypeBody>,
) -> Result<impl IntoResponse, ApiError> {
    remember(&state, body, hq_db::offers::set_fewer).await
}

/// `GET /api/offers/summary`: for Settings, per type of offer over the last 30 days, how many were
/// shown and how they were answered. Counts and catalog wording only; owner token like every
/// `/api` route, and never part of any model prompt (`docs/security/ADOPTION_DATA.md`).
pub(crate) async fn summary_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let (db, now) = (state.db.clone(), chrono::Utc::now().timestamp());
    let rows =
        tokio::task::spawn_blocking(move || db.with_conn(|c| hq_db::offers::summary(c, now)))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?;
    let types: Vec<_> = rows
        .iter()
        .map(|t| {
            json!({
                "type": t.offer, "says": offer_says(&t.offer),
                "shown": t.shown, "tapped": t.tapped, "not_now": t.not_now,
                "later": t.later, "never": t.never,
            })
        })
        .collect();
    Ok(Json(
        json!({ "days": hq_db::offers::SUMMARY_DAYS, "types": types }),
    ))
}

/// `POST /api/offers/clear`: forgets every choice and the counts behind the summary.
pub(crate) async fn clear_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let (db, now) = (state.db.clone(), chrono::Utc::now().timestamp());
    tokio::task::spawn_blocking(move || db.with_conn(|c| hq_db::offers::clear(c, now)))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    const KIND: &str = "pasted_list";

    fn state() -> (tempfile::TempDir, Arc<WsState>) {
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        (vault, state)
    }

    /// A reply carrying an open offer, the way a finished turn leaves it. Returns thread, message.
    fn offered(state: &WsState, kind: &str) -> (String, String) {
        state
            .db
            .with_conn(|c| {
                let t = hq_db::chat::create_thread(c, "t", "user", "user")?;
                let meta = json!({"offer": {
                    "kind": kind, "line": "I can also do it.", "reply": "Do it.", "send": false
                }});
                let m = hq_db::chat::add_message_with_meta(
                    c,
                    &t.thread_id,
                    "assistant",
                    "ok",
                    Some(&meta),
                )?;
                hq_db::offers::record_shown(c, &m.message_id, kind, 1_800_000_000)?;
                Ok((t.thread_id, m.message_id))
            })
            .unwrap()
    }

    async fn answer(state: &Arc<WsState>, message_id: &str, answer: &str) -> StatusCode {
        let body = AnswerBody {
            message_id: message_id.to_string(),
            answer: answer.to_string(),
            at: None,
        };
        match answer_handler(State(state.clone()), Json(body)).await {
            Ok(r) => r.into_response().status(),
            Err(e) => e.into_response().status(),
        }
    }

    async fn body_of(r: impl IntoResponse) -> serde_json::Value {
        let bytes = to_bytes(r.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn items(state: &WsState) -> Vec<(String, String, String, bool, String)> {
        state
            .db
            .with_conn(|c| {
                let mut stmt =
                    c.prepare("SELECT id, source_task, title, owner_only, kind FROM value_items")?;
                let rows = stmt.query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .unwrap()
    }

    async fn item_action(state: &Arc<WsState>, id: &str, action: &str) -> StatusCode {
        let body = crate::notifications_api::NotificationActionRequest {
            action: action.to_string(),
        };
        crate::notifications_api::notification_action_handler(
            State(state.clone()),
            axum::extract::Path(format!("val_{id}")),
            Json(body),
        )
        .await
        .status()
    }

    fn offers_in(state: &WsState, thread: &str) -> i64 {
        state
            .db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1
                      AND json_extract(meta, '$.offer.kind') IS NOT NULL",
                    [thread],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    #[tokio::test]
    async fn later_leaves_one_quiet_device_only_item_that_the_inbox_can_open() {
        let (_vault, state) = state();
        let (thread, message) = offered(&state, KIND);
        assert_eq!(answer(&state, &message, "later").await, StatusCode::OK);
        assert_eq!(
            answer(&state, &message, "later").await,
            StatusCode::NOT_FOUND,
            "an answered offer cannot be parked twice"
        );

        let all = items(&state);
        assert_eq!(all.len(), 1);
        let (id, source, title, owner_only, kind) = all[0].clone();
        assert_eq!(source, SCHEDULED_OFFER_SOURCE);
        assert_eq!(kind, "fyi");
        assert!(!owner_only, "a Later offer is not a decision");
        assert_eq!(title, "Still want to turn a pasted list into tasks?");
        assert!(later_item(&id, KIND, "t1").is_device_only());

        // The inbox offers `open` ("Bring it back") and dismiss, and nothing else.
        let listed =
            crate::notifications_api::aggregate_notifications(&state.db, None, None, 10).unwrap();
        let n = listed
            .notifications
            .iter()
            .find(|n| n.id == format!("val_{id}"))
            .unwrap();
        assert_eq!(n.actions, vec!["open", "dismiss"]);
        assert_eq!(n.metadata.extra["open_label"], "Bring it back");
        assert_eq!(
            n.metadata.link.as_deref(),
            Some(format!("/chat?thread={thread}").as_str()),
            "the item names the chat the offer comes back to"
        );
        assert_eq!(
            offers_in(&state, &thread),
            0,
            "nothing comes back by itself"
        );

        // The same call the apps make: POST /api/notifications/{id}/action.
        assert_eq!(item_action(&state, &id, "open").await, StatusCode::OK);
        assert_eq!(offers_in(&state, &thread), 1, "back in the chat at once");
        let again = state
            .db
            .with_conn(|c| hq_db::offers::deliver_later(c, 1_900_000_000))
            .unwrap();
        assert_eq!(again, 0, "and only once");
        assert_eq!(
            item_action(&state, &id, "open").await,
            StatusCode::OK,
            "a repeat is not an error"
        );
        assert_eq!(
            offers_in(&state, &thread),
            1,
            "and brings nothing back twice"
        );
    }

    #[tokio::test]
    async fn dismissing_the_later_item_drops_the_offer_and_open_is_for_later_items_only() {
        let (_vault, state) = state();
        let (thread, message) = offered(&state, KIND);
        answer(&state, &message, "later").await;
        let id = items(&state)[0].0.clone();
        assert_eq!(item_action(&state, &id, "dismiss").await, StatusCode::OK);
        let n = state
            .db
            .with_conn(|c| hq_db::offers::deliver_later(c, 1_900_000_000))
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(offers_in(&state, &thread), 0);

        // `open` on any other update would be an approval, so it is refused.
        let other = hq_core::types::ValueItem::new(
            SCHEDULED_OFFER_SOURCE,
            ValueKind::Fyi,
            "A step you scheduled is ready",
            "b",
        );
        hq_db::value_items::insert(&state.db, &other).unwrap();
        assert_eq!(
            item_action(&state, &other.id, "open").await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn an_offer_that_came_back_cannot_be_set_aside_again() {
        let (_vault, state) = state();
        let (thread, message) = offered(&state, KIND);
        answer(&state, &message, "later").await;
        let id = items(&state)[0].0.clone();
        item_action(&state, &id, "open").await;
        let back: String = state
            .db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT message_id FROM chat_messages WHERE thread_id = ?1
                      AND json_extract(meta, '$.offer.kind') IS NOT NULL
                      ORDER BY rowid DESC LIMIT 1",
                    [&thread],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(answer(&state, &back, "later").await, StatusCode::CONFLICT);
        assert_eq!(answer(&state, &back, "not_now").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_second_later_item_never_appears_when_the_parking_fails() {
        let (_vault, state) = state();
        let (_thread, message) = offered(&state, KIND);
        // Make the inbox refuse inserts: the item cannot be written, so nothing may be marked answered.
        state
            .db
            .with_conn(|c| {
                Ok(c.execute_batch(
                    "CREATE TRIGGER no_items BEFORE INSERT ON value_items
                     BEGIN SELECT RAISE(ABORT, 'no inbox'); END",
                )?)
            })
            .unwrap();
        assert_eq!(
            answer(&state, &message, "later").await,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        state
            .db
            .with_conn(|c| Ok(c.execute_batch("DROP TRIGGER no_items")?))
            .unwrap();
        assert_eq!(
            answer(&state, &message, "later").await,
            StatusCode::OK,
            "the offer stayed open, so a retry works"
        );
    }

    #[tokio::test]
    async fn answering_a_later_offer_from_the_chat_brings_it_back_now() {
        let (_vault, state) = state();
        let (thread, message) = offered(&state, KIND);
        answer(&state, &message, "later").await;
        let open = |id: &str| MessageBody {
            message_id: id.to_string(),
        };
        let ok = later_open_handler(State(state.clone()), Json(open(&message)))
            .await
            .map(IntoResponse::into_response);
        assert_eq!(ok.unwrap().status(), StatusCode::OK);
        let again = later_open_handler(State(state.clone()), Json(open(&message))).await;
        assert!(again.is_err(), "once");
        let offers: i64 = state
            .db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1
                      AND json_extract(meta, '$.offer.kind') IS NOT NULL",
                    [&thread],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(offers, 1);
    }

    #[tokio::test]
    async fn not_never_fewer_summary_and_clear() {
        let (_vault, state) = state();
        let (_t, message) = offered(&state, KIND);
        assert_eq!(answer(&state, &message, "not_now").await, StatusCode::OK);
        assert_eq!(
            answer(&state, &message, "maybe").await,
            StatusCode::BAD_REQUEST
        );

        let never = |t: &str| TypeBody {
            r#type: t.to_string(),
        };
        let bad = never_handler(State(state.clone()), Json(never("not a type"))).await;
        assert!(bad.is_err(), "only real types are remembered");
        never_handler(State(state.clone()), Json(never("find-notes")))
            .await
            .unwrap();
        fewer_handler(State(state.clone()), Json(never("dated_todo")))
            .await
            .unwrap();

        let v = body_of(summary_handler(State(state.clone())).await.unwrap()).await;
        assert_eq!(v["days"], 30);
        let types = v["types"].as_array().unwrap();
        let get = |t: &str| types.iter().find(|x| x["type"] == t).cloned().unwrap();
        let row = get("tasks-from-list");
        assert_eq!(row["shown"], 1, "a kind is counted under its capability");
        assert_eq!(row["not_now"], 1);
        assert_eq!(row["tapped"], 0);
        assert_eq!(get("find-notes")["never"], 1);
        assert!(
            !v.to_string().contains("Do it."),
            "counts and catalog words only, never a message"
        );

        clear_handler(State(state.clone())).await.unwrap();
        let v = body_of(summary_handler(State(state.clone())).await.unwrap()).await;
        assert_eq!(v["types"].as_array().unwrap().len(), 0);
        let hidden = state
            .db
            .with_conn(|c| hq_db::offers::muted(c, 4_000_000_000))
            .unwrap();
        assert!(hidden.is_empty());
    }

    #[tokio::test]
    async fn no_offer_is_chosen_in_a_voice_turn_or_while_an_approval_waits() {
        let (_vault, state) = state();
        let install = Install {
            model: true,
            ocr: true,
            email: true,
            helpers: true,
        };
        let ctx = OfferContext {
            asked_at: 0,
            text: "Remind me to pay rent on Friday".into(),
            registry: None,
            voice: false,
        };
        let pick =
            |turn: TurnState| rule_offer(state.db.clone(), install, ctx.clone(), vec![], turn);
        // The positive control: with nothing in the way, the same message gets an offer.
        let calm = pick(TurnState::default()).await.expect("allowed");
        assert_eq!(calm.expect("a ready install offers").kind, "dated_todo");
        for blocked in [
            TurnState {
                voice: true,
                ..Default::default()
            },
            TurnState {
                approval_pending: true,
                ..Default::default()
            },
            TurnState {
                errored: true,
                ..Default::default()
            },
        ] {
            assert!(pick(blocked).await.is_none(), "{blocked:?}");
        }
    }
}
