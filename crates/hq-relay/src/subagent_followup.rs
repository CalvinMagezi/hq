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
    // A follow-up runs unattended with the owner's tools and no guest role, so
    // it never runs in a guest's chat; the owner's inbox gets it instead.
    if !(surface.owner_chat)(key) {
        followup::refused(
            db,
            &f,
            "it was started from a guest's chat, where HQ does not run follow-up turns",
        );
        return;
    }
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
        // HQ wrote this prompt and nobody is in the chat to approve anything the turn does.
        unattended: true,
        prompt_by_hq: true,
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

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::CollaborationConfig;
    use hq_core::types::ValueKind;
    use hq_db::subagent_runs::{self as runs, NewRun, Origin, Settlement};
    use std::collections::HashMap;
    use tokio::sync::Mutex as TokioMutex;

    fn settled_run(db: &Database, run_id: &str, chat: &str) {
        let now = chrono::Utc::now().timestamp();
        db.with_conn(|c| {
            runs::insert_run(
                c,
                &NewRun {
                    run_id: run_id.into(),
                    parent_run_id: None,
                    parent_turn_id: None,
                    origin: Some(Origin {
                        platform: "telegram".into(),
                        chat_id: chat.into(),
                        thread_id: None,
                        identity: None,
                    }),
                    task_id: None,
                    child_id: "c".into(),
                    role: "general".into(),
                    goal: "look something up".into(),
                    success_criteria: vec![],
                    required_tools: vec![],
                    detached: true,
                    owner_pid: Some(1),
                    followup_depth: 0,
                },
                now,
            )?;
            runs::settle(
                c,
                run_id,
                &Settlement {
                    exec_status: runs::EXEC_COMPLETED.into(),
                    accept_status: runs::ACCEPT_UNVERIFIED.into(),
                    missing_deliverables: vec![],
                    blocker_reason: None,
                    next_action: None,
                    output_full: "ok".into(),
                    output_preview: "ok".into(),
                    error: None,
                    resolved_backend: "hq".into(),
                },
                now,
                true,
            )
            .map(|_| ())
        })
        .unwrap();
    }

    // A follow-up turn carries the owner's tools and no guest role, so in a
    // guest's chat it must never start; the owner's inbox hears about it.
    #[tokio::test]
    async fn a_followup_for_a_guests_chat_never_runs_and_reaches_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(&dir.path().join("vault.db")).unwrap());
        settled_run(&db, "run-guest111", "42");
        let cfg = CollaborationConfig {
            supervision_followup: true,
            ..Default::default()
        };
        let f = followup::claim_followups(&db, &cfg, "telegram", "w").remove(0);
        let threads = Arc::new(TokioMutex::new(HashMap::new()));
        let sent = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let surface = WatchSurface {
            platform: "telegram",
            threads: threads.clone(),
            system_prompt: Arc::new(String::new()),
            progress_chars: 100,
            identity: hq_core::identity::RequestIdentity::from_telegram,
            sender: {
                let sent = sent.clone();
                Arc::new(move |_key: i64| -> crate::relay_common::ChatSender {
                    let sent = sent.clone();
                    Arc::new(move |text| sent.lock().unwrap().push(text))
                })
            },
            owner_chat: Arc::new(|key: i64| key != 42),
        };
        deliver(&surface, &db, f).await;
        assert!(
            threads.lock().await.is_empty(),
            "the chat was never claimed"
        );
        assert!(sent.lock().unwrap().is_empty());
        assert!(
            followup::claim_followups(&db, &cfg, "telegram", "w").is_empty(),
            "not retried"
        );
        let item = hq_db::value_items::find_active_by_dedup(
            &db,
            ValueKind::ActionNeeded,
            "subagent-run-run-guest111",
        )
        .unwrap();
        assert!(item.is_some(), "the owner's inbox has it");
    }
}
