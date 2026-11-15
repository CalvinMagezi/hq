//! Keeps the driven-session slots in use: gives back the slot of a session that sat idle, and
//! lets the oldest session the cap held back take a free one. Runs at the start of every
//! drive pass, so a session that finished, exited or was released is replaced within one pass.

use super::*;

/// Both halves with the numbers passed in, so a test needs no config file.
pub(super) fn rebalance_with(state: &WsState, cap: i64, idle: Option<Duration>) {
    if let Some(idle) = idle {
        release_idle(state, idle);
    }
    promote_waiting(state, cap);
}

pub(super) fn rebalance(state: &WsState, cfg: &hq_core::config::AgentHostConfig) {
    rebalance_with(state, cfg.driven_session_cap(), cfg.driver_idle_release());
}

fn release_idle(state: &WsState, idle: Duration) {
    let minutes = (idle.as_secs() / 60).max(1);
    let released = match state.db.with_conn(|c| registry::release_idle(c, minutes)) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "session-driver: could not release idle slots");
            return;
        }
    };
    for id in released {
        notify(
            state,
            &id,
            "released",
            &format!(
                "**Drive released for session {id}.** It sat idle for {minutes} minutes, so its slot is free for another session. The session keeps running; switch Drive back on in the Watching panel to continue."
            ),
        );
    }
}

fn promote_waiting(state: &WsState, cap: i64) {
    let candidates = match state.db.with_conn(registry::promotable) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "session-driver: could not list sessions waiting for a slot");
            return;
        }
    };
    for row in candidates {
        let free = state
            .db
            .with_conn(registry::count_driven_running)
            .is_ok_and(|n| n < cap);
        if !free {
            return;
        }
        let open = row
            .owner_thread
            .as_deref()
            .is_some_and(|t| chat_is_open(state, t));
        if !open {
            continue;
        }
        if matches!(
            state.db.with_conn(|c| registry::promote(c, &row.id, cap)),
            Ok(true)
        ) {
            let text = format!(
                "**Drive switched on for session {}.** A slot freed up and this session had been waiting for one with a goal that passes the gate.",
                session_name(&row)
            );
            notify(state, &row.id, "promoted", &text);
        }
    }
}

/// One fixed-text notice in the session's chat. It never wakes the driver.
fn notify(state: &WsState, session_id: &str, slot: &str, text: &str) {
    let Some(thread) = state
        .db
        .with_conn(|c| registry::get(c, session_id))
        .ok()
        .flatten()
        .and_then(|r| r.owner_thread)
    else {
        return;
    };
    crate::ws::post_assistant_message(
        state,
        &thread,
        text,
        &json!({"driver": {"session_id": session_id, "mode": MODE_UPDATE, "slot": slot}}),
    );
    broadcast_sync(state, &thread);
}
