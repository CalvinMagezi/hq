//! Watch scheduler: re-dispatches due recurring watch turns (kind='watch').
//!
//! One scheduler task per relay surface, spawned alongside the platform's
//! listener. Every poll asks the background-turn registry for due watches,
//! filters to this surface's platform, then for each row either closes it out
//! (`watch_until` reached) or re-dispatches the stored prompt through the
//! surface's normal dispatch closure so the firing detaches and delivers like
//! a normal turn.
//!
//! Crash safety: `watch_last_fired` is marked BEFORE dispatch, so a crash
//! mid-dispatch loses at most one firing instead of double-firing. Because the
//! registry is the source of truth, watches survive restarts by construction —
//! the first poll after startup fires everything that came due while down.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use hq_core::identity::RequestIdentity;
use hq_db::Database;
use hq_db::background_turns::{self, BackgroundTurnRow};
use tokio::sync::Mutex as TokioMutex;

use crate::native_run::{ChatKey, TurnRow};
use crate::relay_common::{ChannelState, ChatSender};

/// Returned by a dispatch whose chat already has a turn running. The tick puts
/// the watch's fire time back so the firing is retried on the next poll instead
/// of waiting out a whole interval.
#[derive(Debug)]
pub(crate) struct ChatBusy;

impl std::fmt::Display for ChatBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("chat busy")
    }
}

impl std::error::Error for ChatBusy {}

/// Default poll cadence for the watch scheduler.
pub const DEFAULT_POLL_SECS: u64 = 30;

/// Boxed future returned by the surface callbacks. `Ok(())` means the callback
/// did its best; `Err` is logged by the scheduler and the loop moves on — one
/// bad watch must never stall the others.
pub type WatchFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Re-dispatch one due watch's stored prompt through the surface's normal
/// dispatch path (the same engine the `/watch` first firing flows through).
/// The closure captures whatever the surface needs (bot/http handle, config,
/// system prompt) and is responsible for delivering results to the chat.
pub type WatchDispatch = Arc<dyn Fn(BackgroundTurnRow) -> WatchFuture + Send + Sync>;

/// Deliver a plain chat message about a watch (expiry notice). Separate from
/// `WatchDispatch` so expiry never spins up a harness turn.
pub type WatchNotify = Arc<dyn Fn(BackgroundTurnRow, String) -> WatchFuture + Send + Sync>;

/// Spawnable scheduler loop: polls every `poll_every` until the task is
/// aborted (the relay owns the task handle). Poll-level db errors are logged
/// and retried on the next tick; per-watch errors are logged inside `tick`
/// and never stop the loop.
pub async fn run_watch_scheduler(
    db: Arc<Database>,
    platform: &'static str,
    poll_every: Duration,
    dispatch: WatchDispatch,
    notify: WatchNotify,
) {
    let mut interval = tokio::time::interval(poll_every);
    // The first `interval` tick resolves immediately; consume it so the first
    // real poll lands one full period after startup.
    interval.tick().await;
    loop {
        interval.tick().await;
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = tick(&db, platform, now, &dispatch, &notify).await {
            tracing::warn!(%e, platform, "watch scheduler: poll failed, retrying next tick");
        }
    }
}

/// Characters of a watch's last result quoted in its expiry notice.
const EXPIRY_RESULT_EXCERPT_CHARS: usize = 300;

/// Final status for an expired watch: its short ref, prompt, and last result when one exists.
fn expiry_notice(row: &BackgroundTurnRow) -> String {
    let head = format!(
        "Watch `{}` expired: '{}'.",
        background_turns::short_ref(&row.id),
        crate::relay_common::resume_excerpt(&row.prompt, 60)
    );
    match row.result_text.as_deref().map(str::trim) {
        Some(last) if !last.is_empty() => format!(
            "{head} Last result: {}",
            crate::relay_common::resume_excerpt(last, EXPIRY_RESULT_EXCERPT_CHARS)
        ),
        _ => format!("{head} It never produced a result."),
    }
}

/// One scheduler pass over due watches for `platform`. Pure-ish: all platform
/// I/O happens through the injected closures, so unit tests drive this with an
/// in-memory db and recording closures.
///
/// Per watch:
/// - expired (`watch_until <= now`): mark completed with the expiry notice
///   (which quotes the last result) and send it; no dispatch.
/// - otherwise: mark fired at `now` FIRST (a mark failure skips dispatch to
///   avoid a double-fire on the next poll), then dispatch the stored prompt.
pub async fn tick(
    db: &Database,
    platform: &str,
    now: i64,
    dispatch: &WatchDispatch,
    notify: &WatchNotify,
) -> Result<()> {
    let due = db.with_conn(|c| background_turns::list_due_watches(c, now))?;
    for row in due {
        // The registry query spans all platforms; this scheduler only owns
        // its own surface's watches.
        if row.platform != platform {
            continue;
        }
        if row.watch_until.is_some_and(|until| until <= now) {
            let notice = expiry_notice(&row);
            if let Err(e) =
                db.with_conn(|c| background_turns::mark_completed(c, &row.id, &notice, now))
            {
                tracing::warn!(%e, watch = %row.id, "watch scheduler: expiry close-out failed");
                continue;
            }
            if let Err(e) = notify(row.clone(), notice).await {
                tracing::warn!(%e, watch = %row.id, "watch scheduler: expiry notice failed");
            }
            continue;
        }
        // A watch that follows a task ends when the task does, instead of firing on.
        if let Some(reason) = db.with_conn(|c| background_turns::watch_finished_reason(c, &row.id)).ok().flatten() {
            if let Err(e) = db.with_conn(|c| background_turns::mark_completed(c, &row.id, &reason, now)) {
                tracing::warn!(%e, watch = %row.id, "watch scheduler: closing a watch whose task ended failed");
                continue;
            }
            // The chat that set the watch is told it ended, as for an expired one.
            let notice = format!("Watch `{}` stopped. {reason}", background_turns::short_ref(&row.id));
            if let Err(e) = notify(row.clone(), notice).await {
                tracing::warn!(%e, watch = %row.id, "watch scheduler: task-ended notice failed");
            }
            continue;
        }
        if let Err(e) = db.with_conn(|c| background_turns::mark_watch_fired(c, &row.id, now)) {
            // Without the fired-mark the next poll would see the watch as due
            // again; skip dispatch rather than risk a double-fire.
            tracing::warn!(%e, watch = %row.id, "watch scheduler: mark_watch_fired failed, skipping dispatch");
            continue;
        }
        match dispatch(row.clone()).await {
            Ok(()) => {}
            Err(e) if e.is::<ChatBusy>() => {
                // NULL and created_at are equivalent for list_due_watches' COALESCE.
                let previous = row.watch_last_fired.unwrap_or(row.created_at);
                if let Err(e) = db.with_conn(|c| background_turns::mark_watch_fired(c, &row.id, previous)) {
                    tracing::warn!(%e, watch = %row.id, "watch scheduler: could not un-mark a busy firing");
                }
                tracing::info!(watch = %row.id, "watch scheduler: chat busy, retrying next tick");
            }
            Err(e) => tracing::warn!(%e, watch = %row.id, "watch scheduler: dispatch failed"),
        }
    }
    Ok(())
}

/// What one relay surface supplies so the shared watch path can fire its watches.
#[derive(Clone)]
pub(crate) struct WatchSurface<K> {
    pub platform: &'static str,
    pub threads: Arc<TokioMutex<HashMap<K, ChannelState>>>,
    pub system_prompt: Arc<String>,
    pub progress_chars: usize,
    pub identity: fn(K) -> RequestIdentity,
    pub sender: Arc<dyn Fn(K) -> ChatSender + Send + Sync>,
}

/// The scheduler's dispatch and notify closures for one surface.
pub(crate) fn watch_callbacks<K: ChatKey>(surface: WatchSurface<K>) -> (WatchDispatch, WatchNotify) {
    let surface = Arc::new(surface);
    let dispatch: WatchDispatch = {
        let surface = surface.clone();
        Arc::new(move |row| {
            let surface = surface.clone();
            Box::pin(async move { dispatch_watch_firing(&surface, row).await })
        })
    };
    let notify: WatchNotify = Arc::new(move |row, text| {
        let surface = surface.clone();
        Box::pin(async move {
            (surface.sender)(parse_chat_key::<K>(&row)?)(text);
            Ok(())
        })
    });
    (dispatch, notify)
}

fn parse_chat_key<K: ChatKey>(row: &BackgroundTurnRow) -> Result<K> {
    row.chat_id
        .parse()
        .map_err(|_| anyhow::anyhow!("watch: bad chat_id '{}'", row.chat_id))
}

/// Take the chat's single-flight slot, or report it busy with a live turn.
pub(crate) async fn claim_chat<K: ChatKey>(threads: &TokioMutex<HashMap<K, ChannelState>>, key: K) -> bool {
    let mut t = threads.lock().await;
    let state = t.entry(key).or_insert_with(ChannelState::new_default);
    if state.turn_in_flight {
        return false;
    }
    state.turn_in_flight = true;
    // A steer inbox left by the previous turn would make a user message look
    // steered into this firing, which has no inbox, and the message would be lost.
    state.pending_steer = None;
    true
}

/// One watch firing. A busy chat defers the firing to the next tick rather than
/// running a second turn against the same history.
async fn dispatch_watch_firing<K: ChatKey>(
    surface: &WatchSurface<K>,
    row: BackgroundTurnRow,
) -> Result<()> {
    let key = parse_chat_key::<K>(&row)?;
    if !claim_chat(&surface.threads, key).await {
        return Err(ChatBusy.into());
    }
    let result = run_watch_firing(surface, key, &row).await;
    if let Some(state) = surface.threads.lock().await.get_mut(&key) {
        state.turn_in_flight = false;
    }
    result
}

async fn run_watch_firing<K: ChatKey>(
    surface: &WatchSurface<K>,
    key: K,
    row: &BackgroundTurnRow,
) -> Result<()> {
    let config = crate::native_run::load_config("watch dispatch");
    let (session, repo_root) = crate::native_run::relay_session(&config, false);
    let instructions = format!(
        "{}\n\n## Scheduled watch firing\nThis turn is watch `{}` firing on its interval; the user is not watching live. Do the check and report findings concisely. Anything you return is delivered to the chat when the turn finishes. {}",
        crate::session_runner::strip_loaded_soul(&surface.system_prompt, &config.vault_path),
        row.id,
        crate::relay_common::WATCH_DONE_INSTRUCTION,
    );

    // Each firing owns a fresh kind='turn' row; the watch row only tracks cadence.
    let turn = TurnRow::register(&config, surface.platform, &row.chat_id, row.identity.as_deref(), &row.prompt);
    let send = (surface.sender)(key);
    let on_detached: hq_agent::native_hq::DetachedTurnSink = {
        let (db, send, watch_id) = (turn.db.clone(), send.clone(), row.id.clone());
        Arc::new(move |outcome: hq_agent::native_hq::DetachedTurnOutcome| {
            let (text, _) =
                crate::relay_common::settle_watch_firing(db.as_deref(), &watch_id, &outcome.text);
            let outcome = hq_agent::native_hq::DetachedTurnOutcome { text, ..outcome };
            crate::relay_common::finish_detached(db.as_deref(), &send, &outcome);
        })
    };
    let permission_preset = surface
        .threads
        .lock()
        .await
        .get(&key)
        .and_then(|s| s.pinned_permission_preset);

    let run = hq_agent::native_hq::run_native_hq(
        &config,
        &row.prompt,
        instructions,
        repo_root,
        session,
        hq_agent::native_hq::NativeHqHooks {
            on_detached: Some(on_detached),
            permission_preset,
            ..turn.hooks(&config, (surface.identity)(key), send.clone(), surface.progress_chars)
        },
    )
    .await;

    match run {
        Ok(result) => {
            tracing::info!(watch = %row.id, detached = result.detached, "watch dispatch: firing completed/parked");
            // Detached firings are closed and delivered by `on_detached`.
            if !result.detached {
                settle_inline_firing(turn.db.as_deref(), &turn.id, row, &result, &send);
            }
            Ok(())
        }
        Err(e) => {
            crate::relay_common::close_turn_row(turn.db.as_deref(), &turn.id, &e.to_string(), false);
            Err(anyhow::anyhow!("watch dispatch: run_native_hq failed: {e}"))
        }
    }
}

/// Close a firing that finished inside the ack window and deliver it when it is
/// news. Without this its row leaks as 'running' and the result never arrives.
fn settle_inline_firing(
    db: Option<&Database>,
    turn_id: &str,
    row: &BackgroundTurnRow,
    result: &hq_agent::native_hq::NativeHqResult,
    send: &ChatSender,
) {
    let (text, done) = crate::relay_common::settle_watch_firing(db, &row.id, &result.text);
    crate::relay_common::close_turn_row(db, turn_id, &text, result.success);
    // Compared with the watch row's cached result so a healthy watch repeating
    // itself for up to 30 days stays quiet. A done reply is the watch's last word.
    let changed = row
        .result_text
        .as_deref()
        .is_none_or(|prev| prev.trim() != text.trim());
    let should_deliver = done || !result.success || changed;
    if let Some(db) = db
        && let Err(e) = db.with_conn(|c| background_turns::update_watch_result(c, &row.id, &text))
    {
        tracing::warn!(%e, watch = %row.id, "watch dispatch: caching last result failed");
    }
    if !should_deliver {
        tracing::debug!(watch = %row.id, "watch dispatch: unchanged result, delivery skipped");
    } else if !text.trim().is_empty() {
        send(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const T0: i64 = 1_700_000_000;

    fn seed_watch(db: &Database, id: &str, platform: &str, interval: i64, until: Option<i64>) {
        db.with_conn(|c| {
            background_turns::insert_watch(
                c,
                id,
                platform,
                "chat-42",
                None,
                Some("alex"),
                "check whether the review is done",
                T0,
                interval,
                until,
            )
        })
        .unwrap();
    }

    fn recording_dispatch(
        log: Arc<Mutex<Vec<String>>>,
        fail_ids: Arc<Mutex<Vec<String>>>,
    ) -> WatchDispatch {
        Arc::new(move |row| {
            let log = log.clone();
            let fail_ids = fail_ids.clone();
            Box::pin(async move {
                log.lock().unwrap().push(row.id.clone());
                if fail_ids.lock().unwrap().contains(&row.id) {
                    Err(anyhow::anyhow!("dispatch boom"))
                } else {
                    Ok(())
                }
            })
        })
    }

    fn recording_notify(log: Arc<Mutex<Vec<(String, String)>>>) -> WatchNotify {
        Arc::new(move |row, text| {
            let log = log.clone();
            Box::pin(async move {
                log.lock().unwrap().push((row.id.clone(), text));
                Ok(())
            })
        })
    }

    #[tokio::test]
    async fn busy_chat_leaves_the_watch_due_for_the_next_tick() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-busy", "telegram", 60, None);
        let busy: WatchDispatch = Arc::new(|_row| Box::pin(async { Err(ChatBusy.into()) }));

        tick(&db, "telegram", T0 + 100, &busy, &recording_notify(Arc::new(Mutex::new(vec![]))))
            .await
            .unwrap();

        let due = db.with_conn(|c| background_turns::list_due_watches(c, T0 + 130)).unwrap();
        assert_eq!(due.len(), 1, "a busy firing must be retried, not skipped for an interval");
    }

    #[tokio::test]
    async fn a_watch_whose_task_is_complete_is_closed_instead_of_fired() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Work", "work", "WK")?;
            hq_db::tasks::create_task(c, "tk-1", "in-1", &hq_db::tasks::NewTask { title: "Ship", created_by: "t", ..Default::default() })?;
            Ok(())
        })
        .unwrap();
        seed_watch(&db, "w-task", "telegram", 60, None);
        db.with_conn(|c| background_turns::set_watch_task(c, "w-task", "WK-001")).unwrap();
        db.with_conn(|c| {
            hq_db::tasks::update_task(c, "tk-1", &hq_db::tasks::TaskPatch { status: Some("complete".into()), ..Default::default() }, None)
                .map(|_| ())
        })
        .unwrap();

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(dispatched.clone(), Arc::new(Mutex::new(vec![]))),
            &recording_notify(Arc::new(Mutex::new(vec![]))),
        )
        .await
        .unwrap();

        assert!(dispatched.lock().unwrap().is_empty(), "no firing for a finished task");
        let row = db.with_conn(|c| background_turns::get(c, "w-task")).unwrap().unwrap();
        assert_eq!(row.status, "completed");
        assert!(row.result_text.unwrap().contains("complete, so this watch stopped"));
        let thread = db.with_conn(|c| hq_db::tasks::list_comments(c, "tk-1")).unwrap();
        assert!(thread.iter().any(|c| c.body.starts_with("Watch finished")), "{thread:?}");
    }

    #[tokio::test]
    async fn due_watch_dispatches_and_marks_fired() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-1", "telegram", 60, None);

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(dispatched.clone(), Arc::new(Mutex::new(vec![]))),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();

        assert_eq!(*dispatched.lock().unwrap(), vec!["w-1".to_string()]);
        assert!(notified.lock().unwrap().is_empty());
        let row = db
            .with_conn(|c| background_turns::get(c, "w-1"))
            .unwrap()
            .unwrap();
        assert_eq!(row.watch_last_fired, Some(T0 + 100));
        assert_eq!(row.status, background_turns::STATUS_RUNNING);
        // No longer due at the same instant.
        assert!(
            db.with_conn(|c| background_turns::list_due_watches(c, T0 + 100))
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn expired_watch_completes_and_notifies() {
        let db = Database::open_memory().unwrap();
        // Due (interval elapsed) AND expired (watch_until already passed).
        seed_watch(&db, "w-1", "telegram", 60, Some(T0 + 50));

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(dispatched.clone(), Arc::new(Mutex::new(vec![]))),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();

        let expected =
            "Watch `w-1` expired: 'check whether the review is done'. It never produced a result.";
        assert!(dispatched.lock().unwrap().is_empty());
        assert_eq!(
            *notified.lock().unwrap(),
            vec![("w-1".to_string(), expected.to_string())]
        );
        let row = db
            .with_conn(|c| background_turns::get(c, "w-1"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, background_turns::STATUS_COMPLETED);
        assert_eq!(row.result_text.as_deref(), Some(expected));
        assert_eq!(row.completed_at, Some(T0 + 100));
    }

    #[tokio::test]
    async fn expiry_notice_quotes_the_last_result() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "abcdef1234", "telegram", 60, Some(T0 + 50));
        db.with_conn(|c| {
            background_turns::update_watch_result(c, "abcdef1234", "review: 2 approvals")
        })
        .unwrap();
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![]))),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();
        let notice = notified.lock().unwrap()[0].1.clone();
        assert!(notice.starts_with("Watch `abcdef12` expired"), "{notice}");
        assert!(
            notice.ends_with("Last result: review: 2 approvals"),
            "{notice}"
        );
    }

    #[tokio::test]
    async fn non_due_watch_is_untouched() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-1", "telegram", 3600, None); // due at T0 + 3600

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(dispatched.clone(), Arc::new(Mutex::new(vec![]))),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();

        assert!(dispatched.lock().unwrap().is_empty());
        assert!(notified.lock().unwrap().is_empty());
        let row = db
            .with_conn(|c| background_turns::get(c, "w-1"))
            .unwrap()
            .unwrap();
        assert!(row.watch_last_fired.is_none());
        assert_eq!(row.status, background_turns::STATUS_RUNNING);
    }

    #[tokio::test]
    async fn one_failing_dispatch_does_not_stop_later_watches() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-1", "telegram", 60, None);
        seed_watch(&db, "w-2", "telegram", 60, None);

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(
                dispatched.clone(),
                Arc::new(Mutex::new(vec!["w-1".to_string()])),
            ),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();

        // Both attempted (oldest first), both marked fired despite w-1 failing.
        assert_eq!(
            *dispatched.lock().unwrap(),
            vec!["w-1".to_string(), "w-2".to_string()]
        );
        for id in ["w-1", "w-2"] {
            let row = db
                .with_conn(|c| background_turns::get(c, id))
                .unwrap()
                .unwrap();
            assert_eq!(row.watch_last_fired, Some(T0 + 100));
        }
    }

    #[tokio::test]
    async fn other_platforms_watches_are_skipped() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-discord", "discord", 60, None);

        let dispatched = Arc::new(Mutex::new(Vec::new()));
        let notified = Arc::new(Mutex::new(Vec::new()));
        tick(
            &db,
            "telegram",
            T0 + 100,
            &recording_dispatch(dispatched.clone(), Arc::new(Mutex::new(vec![]))),
            &recording_notify(notified.clone()),
        )
        .await
        .unwrap();

        assert!(dispatched.lock().unwrap().is_empty());
        let row = db
            .with_conn(|c| background_turns::get(c, "w-discord"))
            .unwrap()
            .unwrap();
        assert!(row.watch_last_fired.is_none());
    }

    #[tokio::test]
    async fn busy_chat_defers_the_firing_and_keeps_the_live_turns_slot() {
        let db = Database::open_memory().unwrap();
        seed_watch(&db, "w-busy", "discord", 60, None);
        let mut row = db
            .with_conn(|c| background_turns::get(c, "w-busy"))
            .unwrap()
            .unwrap();
        row.chat_id = "42".to_string();

        let mut busy = ChannelState::new_default();
        busy.turn_in_flight = true;
        let threads = Arc::new(TokioMutex::new(HashMap::from([(42u64, busy)])));
        let sent = Arc::new(Mutex::new(Vec::<String>::new()));
        let sender = {
            let sent = sent.clone();
            Arc::new(move |_key: u64| -> ChatSender {
                let sent = sent.clone();
                Arc::new(move |text| sent.lock().unwrap().push(text))
            })
        };
        let (dispatch, _) = watch_callbacks(WatchSurface {
            platform: "discord",
            threads: threads.clone(),
            system_prompt: Arc::new(String::new()),
            progress_chars: 100,
            identity: RequestIdentity::from_discord,
            sender,
        });

        let err = dispatch(row).await.unwrap_err();
        assert!(err.is::<ChatBusy>());
        assert!(threads.lock().await[&42].turn_in_flight);
        assert!(sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn claim_chat_takes_a_free_slot_once() {
        let threads = TokioMutex::new(HashMap::<i64, ChannelState>::new());
        threads.lock().await.entry(7).or_insert_with(ChannelState::new_default).pending_steer =
            Some(Arc::new(std::sync::Mutex::new(None)));
        assert!(claim_chat(&threads, 7).await);
        assert!(threads.lock().await[&7].pending_steer.is_none());
        assert!(!claim_chat(&threads, 7).await);
    }
}
