//! Telegram and Discord half of the sub-agent supervision loop: when a
//! delegated child settles, is interrupted, or goes quiet, HQ resumes the
//! originating chat with a turn that verifies the evidence, without waiting
//! for a message. The claim, prompt and bookkeeping live in
//! `hq_agent::followup`; this only delivers the turn on one relay surface.
//!
//! A chat with a turn already running gets the claim handed back and is
//! retried on the next pass, so a follow-up never runs against the same
//! history as a live reply.

use std::sync::Arc;
use std::time::Duration;

use hq_agent::followup::{self, Followup};
use hq_db::Database;

use crate::native_run::ChatKey;
use crate::watch_scheduler::{WatchSurface, claim_chat};

/// Short enough that a settled child is picked up well inside a minute.
const PASS_EVERY: Duration = Duration::from_secs(10);

const INSTRUCTIONS_TAIL: &str = "\n\n## Sub-agent follow-up turn\nThis turn was started by HQ itself because delegated work reported back. The user is not watching live. Anything you return is delivered to the chat when the turn finishes.";

pub(crate) async fn run_subagent_followups<K: ChatKey>(
    surface: WatchSurface<K>,
    db: Arc<Database>,
) {
    let claimant = format!(
        "{}-{}-{}",
        surface.platform,
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let mut pass = tokio::time::interval(PASS_EVERY);
    loop {
        pass.tick().await;
        let cfg = crate::native_run::load_config("subagent followup").collaboration;
        for f in followup::claim_followups(&db, &cfg, surface.platform, &claimant) {
            deliver(&surface, &db, f).await;
        }
    }
}

async fn deliver<K: ChatKey>(surface: &WatchSurface<K>, db: &Arc<Database>, f: Followup) {
    let Ok(key) = f.origin.chat_id.parse::<K>() else {
        followup::failed(db, &f, "the chat id could not be parsed");
        return;
    };
    if !claim_chat(&surface.threads, key).await {
        followup::busy(db, &f);
        return;
    }
    // Acknowledged before the turn runs, like the web surface: a turn that
    // fails part-way may already have acted, so it is reported, not retried.
    if !followup::delivered(db, &f) {
        release_chat(surface, key).await;
        return;
    }
    // One chat's long turn must not hold up every other chat's follow-up.
    let (surface, db) = (surface.clone(), db.clone());
    tokio::spawn(async move {
        let result = run_turn(&surface, key, &f).await;
        release_chat(&surface, key).await;
        if let Err(e) = result {
            tracing::warn!(%e, chat = %f.origin.chat_id, "subagent followup: turn failed");
            followup::turn_failed(&db, &f);
        }
    });
}

async fn release_chat<K: ChatKey>(surface: &WatchSurface<K>, key: K) {
    if let Some(state) = surface.threads.lock().await.get_mut(&key) {
        state.turn_in_flight = false;
    }
}

async fn run_turn<K: ChatKey>(
    surface: &WatchSurface<K>,
    key: K,
    f: &Followup,
) -> anyhow::Result<()> {
    let config = crate::native_run::load_config("subagent followup");
    let (session, repo_root) = crate::native_run::relay_session(&config, false);
    let instructions = format!(
        "{}{INSTRUCTIONS_TAIL}",
        crate::session_runner::strip_loaded_soul(&surface.system_prompt, &config.vault_path)
    );
    let hooks = hq_agent::native_hq::NativeHqHooks {
        identity: Some((surface.identity)(key)),
        turn_id: Some(f.turn_id.clone()),
        ..Default::default()
    };
    let result = hq_agent::native_hq::run_native_hq(
        &config,
        &f.prompt,
        instructions,
        repo_root,
        session,
        hooks,
    )
    .await?;
    let text = result.text.trim();
    if !text.is_empty() {
        (surface.sender)(key)(text.to_string());
    }
    Ok(())
}
