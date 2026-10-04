//! Web half of the sub-agent supervision loop: when a delegated child settles,
//! is interrupted, or goes quiet, HQ resumes the originating chat with a turn
//! that reads the evidence and verifies it, without waiting for a message.
//!
//! Events are claimed before acting, so two passes or two processes never
//! start the same follow-up twice. A chat with a reply already running gets
//! the claim handed back and is retried on the next pass.

use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use hq_agent::followup::{self, Followup};

use crate::WsState;

/// Short enough that a settled child is picked up well inside a minute.
const PASS_EVERY: Duration = Duration::from_secs(10);
const PLATFORM: &str = "web";
const MODE: &str = "subagent_followup";

pub(crate) fn spawn_subagent_followup(state: Arc<WsState>) {
    let claimant = format!("web-{}-{}", std::process::id(), uuid::Uuid::new_v4());
    tokio::spawn(async move {
        let mut pass = tokio::time::interval(PASS_EVERY);
        loop {
            pass.tick().await;
            run_pass(&state, &claimant).await;
        }
    });
}

fn collaboration(state: &WsState) -> hq_core::config::CollaborationConfig {
    hq_core::config::HqConfig::load()
        .ok()
        .map(|c| c.collaboration)
        .or_else(|| state.hq_config.as_ref().map(|c| c.collaboration.clone()))
        .unwrap_or_default()
}

async fn run_pass(state: &Arc<WsState>, claimant: &str) {
    let cfg = collaboration(state);
    for f in followup::claim_followups(&state.db, &cfg, PLATFORM, claimant) {
        deliver(state, f).await;
    }
}

async fn deliver(state: &Arc<WsState>, f: Followup) {
    let thread = f.origin.chat_id.clone();
    if !crate::session_driver::chat_is_open(state, &thread) {
        followup::failed(&state.db, &f, "the chat is archived or gone");
        return;
    }
    let meta = json!({
        "mode": MODE,
        "turn_id": f.turn_id,
        "run_ids": f.items.iter().map(|(_, r)| r.run_id.clone()).collect::<Vec<_>>(),
    });
    match crate::ws::start_driver_turn(state, &thread, f.prompt.clone(), meta).await {
        crate::ws::DriverStart::Started => {
            let _ = followup::delivered(&state.db, &f);
            crate::session_driver::broadcast_sync(state, &thread);
        }
        crate::ws::DriverStart::Busy => followup::busy(&state.db, &f),
        crate::ws::DriverStart::Refused => {
            followup::failed(&state.db, &f, "the chat belongs to a read-only question from an MCP client")
        }
    }
}
