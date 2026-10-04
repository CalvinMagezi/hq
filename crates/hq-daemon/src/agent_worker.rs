//! ingest-event processing worker — the "agents reason about events" layer.
//!
//! Drains the `agent-worker` mailbox on its own 20s loop. For each event it:
//!   1. gate-filters (drops learned noise BEFORE spending any tokens),
//!   2. reasons about it via the LLM router (terse triage),
//!   3. writes a result note to `.vault/_events/<date>/`,
//!   4. emits a gated summary to the relay (header preserved so the poller's gate
//!      re-applies the same trait and delivers as digest/urgent).
//!
//! This turns raw inbound events into agent-reasoned, vault-recorded, gated
//! notifications instead of raw forwards.

use hq_core::mailbox;
use crate::notif_gate::{GateDecision, gate_decision, parse_event_header};
use chrono::Utc;
use hq_core::types::{ChatMessage, MailboxMessage, MailboxMessageType, MessageRole};
use hq_llm::{LlmProvider, provider::ChatRequest, router::LlmRouter};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const WORKER_MAILBOX: &str = "agent-worker";
const REASON_TIMEOUT_SECS: u64 = 60;
/// How often the dedicated worker loop drains the mailbox. Short enough for
/// near-real-time event handling, long enough to be negligible when idle.
const WORKER_LOOP_INTERVAL_SECS: u64 = 20;

/// Dedicated background loop for event reasoning. Spawned once at daemon start
/// so it runs independently of the sequential task scheduler, and slow tasks
/// (memory consolidation, embeddings) can never delay inbound events.
pub async fn run_agent_worker_loop(vault_path: PathBuf) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(WORKER_LOOP_INTERVAL_SECS));
    info!(
        interval_secs = WORKER_LOOP_INTERVAL_SECS,
        "agent-worker: dedicated reasoning loop started"
    );
    loop {
        interval.tick().await;
        match run_agent_worker_cycle(&vault_path).await {
            Ok(n) if n > 0 => info!(processed = n, "agent-worker: reasoned events"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "agent-worker: cycle failed"),
        }
    }
}

/// Drain and process the worker mailbox. Returns the number of events reasoned
/// about (drops are skipped without spending tokens and don't count).
pub async fn run_agent_worker_cycle(vault_path: &Path) -> anyhow::Result<usize> {
    let msgs = mailbox::receive_messages(vault_path, WORKER_MAILBOX)?;
    if msgs.is_empty() {
        return Ok(0);
    }
    let router = LlmRouter::from_env();
    let mut processed = 0;
    for msg in msgs {
        match process_event(vault_path, &router, &msg).await {
            Ok(true) => processed += 1,
            Ok(false) => info!(from = %msg.from, "agent-worker: event gated as noise, skipped"),
            Err(e) => {
                // receive_messages already deleted the file; re-enqueue so a
                // transient failure (disk, relay) retries instead of losing the event.
                let _ = mailbox::send_message(vault_path, &msg);
                warn!(error = %e, from = %msg.from, "agent-worker: processing failed, requeued");
            }
        }
    }
    Ok(processed)
}

/// Returns `Ok(true)` if the event was reasoned about and emitted,
/// `Ok(false)` if it was gated as noise (no reasoning, no notification).
async fn process_event(
    vault_path: &Path,
    router: &LlmRouter,
    msg: &MailboxMessage,
) -> anyhow::Result<bool> {
    // Drop learned noise before spending any tokens.
    if gate_decision(vault_path, msg) == GateDecision::Drop {
        return Ok(false);
    }

    if crate::email_triage::is_email_triage(msg) {
        // The ingest layer only forwards raw email fields; the structured
        // CLASSIFICATION/SUMMARY/DRAFT prompt is built here and must reach the
        // model verbatim, or parse_triage finds no sections.
        let identity = hq_core::config::HqConfig::load().ok().and_then(|cfg| {
            let company_id = msg.meta.get("company_id").map(String::as_str).unwrap_or("");
            let co = if company_id.is_empty() {
                cfg.default_company_config().cloned()
            } else {
                cfg.company_by_id(company_id).cloned()
            }?;
            Some((co.identity.contact_name, co.identity.role, co.name))
        });
        let identity_ref = identity
            .as_ref()
            .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()));
        let prompt = crate::email_triage::build_prompt(msg, identity_ref);
        let drafted = complete(router, "fast", prompt).await.unwrap_or_default();
        write_event_note(vault_path, msg, &drafted)?;
        let decisions = hq_llm::decision::get();
        crate::email_triage::handle(vault_path, msg, &drafted, decisions.as_ref()).await?;
        return Ok(true);
    }

    let summary = reason_about(router, &msg.content)
        .await
        .unwrap_or_else(|| body_after_header(&msg.content).to_string());

    write_event_note(vault_path, msg, &summary)?;

    // Emit a gated notification: keep the header so the relay poller's gate
    // re-applies the same trait, then delivers (digest/urgent) to the user.
    let notif = format!("{}\n\n{}", header_line(&msg.content), summary);
    let out = mailbox::new_message(
        "agent-worker",
        "relay",
        MailboxMessageType::Direct,
        None,
        &notif,
        None,
    );
    mailbox::send_message(vault_path, &out)?;
    Ok(true)
}

async fn reason_about(router: &LlmRouter, content: &str) -> Option<String> {
    let prompt = format!(
        "You are HQ's event triage. An external event arrived:\n\n{content}\n\n\
         In 2-3 short lines: what happened, why it might matter, and any action worth taking. \
         Be terse. No preamble, no restating the event verbatim."
    );
    complete(router, "fast", prompt).await
}

async fn complete(router: &LlmRouter, model: &str, prompt: String) -> Option<String> {
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: prompt,
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        ..Default::default()
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(REASON_TIMEOUT_SECS),
        router.chat(&request),
    )
    .await
    {
        Ok(Ok(r)) => Some(r.message.content.trim().to_string()),
        Ok(Err(e)) => {
            warn!(error = %e, "agent-worker: LLM call failed, forwarding raw body");
            None
        }
        Err(_) => {
            warn!("agent-worker: LLM timed out, forwarding raw body");
            None
        }
    }
}

/// The leading `[…]` header line, or a default if the event had none.
fn header_line(content: &str) -> String {
    content
        .lines()
        .next()
        .filter(|l| l.trim_start().starts_with('['))
        .unwrap_or("[hq-event]")
        .to_string()
}

/// Event body with the leading header line (and its blank line) stripped.
fn body_after_header(content: &str) -> &str {
    match content.split_once("\n\n") {
        Some((first, rest)) if first.trim_start().starts_with('[') => rest,
        _ => content,
    }
}

fn write_event_note(vault_path: &Path, msg: &MailboxMessage, summary: &str) -> anyhow::Result<()> {
    let (source, kind) = parse_event_header(&msg.content)
        .map(|h| (h.source, h.kind))
        .unwrap_or_else(|| ("event".to_string(), "event".to_string()));
    let now = Utc::now();
    let dir = vault_path
        .join("_events")
        .join(now.format("%Y-%m-%d").to_string());
    std::fs::create_dir_all(&dir)?;
    // Include a short msg-id fragment so two events with the same source/kind in
    // the same second don't collide and silently overwrite each other.
    let id_frag = msg.id.get(..8).unwrap_or(&msg.id);
    let fname = format!(
        "{}-{}-{}-{}.md",
        source,
        kind.replace('.', "_"),
        now.timestamp(),
        id_frag
    );
    let note = format!(
        "---\nsource: {source}\nkind: {kind}\nts: {}\n---\n\n# {source} / {kind}\n\n{summary}\n\n## Raw event\n\n```\n{}\n```\n",
        now.to_rfc3339(),
        msg.content
    );
    std::fs::write(dir.join(fname), note)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_model::{UserModel, UserTrait, save_user_model};
    use serde_json::json;
    use tempfile::TempDir;

    fn ev(content: &str) -> MailboxMessage {
        mailbox::new_message(
            "ingest",
            "agent-worker",
            MailboxMessageType::Direct,
            None,
            content,
            None,
        )
    }

    #[test]
    fn header_line_extracts_or_defaults() {
        assert_eq!(
            header_line("[hq-event source=x kind=y]\n\nbody"),
            "[hq-event source=x kind=y]"
        );
        assert_eq!(header_line("no header here"), "[hq-event]");
    }

    #[test]
    fn body_after_header_strips_header() {
        assert_eq!(
            body_after_header("[hq-event source=x kind=y]\n\nthe body"),
            "the body"
        );
        assert_eq!(body_after_header("plain body"), "plain body");
    }

    #[test]
    fn write_event_note_creates_dated_file() {
        let tmp = TempDir::new().unwrap();
        let msg = ev("[hq-event flow=ci source=github kind=ci.failed]\n\nmain red");
        write_event_note(tmp.path(), &msg, "CI failed on main.").unwrap();
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let dir = tmp.path().join("_events").join(&day);
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("source: github"));
        assert!(content.contains("CI failed on main."));
        assert!(content.contains("kind: ci.failed"));
    }

    #[tokio::test]
    async fn noise_event_is_dropped_without_reasoning() {
        let tmp = TempDir::new().unwrap();
        let mut m = UserModel::default();
        m.merge(
            "notif.github.dependabot",
            UserTrait {
                value: json!("noise"),
                confidence: 0.9,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(tmp.path(), &m).unwrap();
        let router = LlmRouter::from_env();
        let msg = ev("[hq-event flow=gh source=github kind=dependabot]\n\nbump dep");
        // gate says Drop → returns false, no _events note, no relay message
        let reasoned = process_event(tmp.path(), &router, &msg).await.unwrap();
        assert!(!reasoned);
        assert!(!tmp.path().join("_events").exists());
        assert_eq!(mailbox::message_count(tmp.path(), "relay").unwrap_or(0), 0);
    }
}
