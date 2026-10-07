//! Session supervisor: reconciles the harness_sessions registry against what
//! Herdr reports every minute. It asks each host once, snapshots the screen of
//! every live session, alerts when one blocks on a dialog, marks sessions whose
//! agent is gone as exited, and surfaces transitions on the value bus so the
//! operator hears about them on Telegram/Discord.
//!
//! A host that cannot be reached (a laptop that is asleep or off the tailnet)
//! says nothing about its sessions, so they are skipped rather than marked
//! exited. They are picked up again the moment the host answers.
//!
//! The value-bus FYI announces the exit but carries no output, which left the
//! operator to ask what the session actually said. On the same transition the
//! supervisor also posts the session's final output to the `relay` mailbox, so
//! the platform bridges deliver the result without being asked. That output is
//! the last screen snapshot taken while the agent was alive: Herdr cannot read
//! a pane that no longer has an agent, so a session that dies before its first
//! sweep has none.
//!
//! A session launched for an HQ task also records each transition on that task
//! (`harness_session::mission`), right after the claim that makes it happen
//! once, so the task stays the durable record across daemon restarts.
//!
//! A session a web chat watches (`owner_thread`) sends nothing to the relay or
//! the value bus: each event becomes a durable wake on its row, and the web
//! server's session driver posts it into that chat or drives the session.

use anyhow::{Result, anyhow, bail};
use hq_core::config::HqConfig;
use hq_core::types::{ChatMessage, MailboxMessageType, MessageRole, ValueItem, ValueKind};
use hq_db::Database;
use hq_db::harness_sessions_registry as registry;
use hq_tools::harness_session::mission::{self, Event};
use hq_tools::harness_session::{Liveness, liveness, poll_hosts_with};
use hq_tools::herdr::{AgentInfo, AgentStatus, Host, HostBackend};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Pane lines kept per stored snapshot.
const SNAPSHOT_LINES: usize = 200;

/// Ceiling on the output excerpt carried in the mailbox body.
const MAX_TAIL_BYTES: usize = 3500;

/// Delimiters around the raw excerpt. A markdown fence would be closed early by
/// the first triple backtick the harness itself printed, and coding harnesses
/// print those constantly.
const OUTPUT_OPEN: &str = "----- session output -----";
const OUTPUT_CLOSE: &str = "----- end session output -----";

const SUMMARY_PROMPT: &str = "Summarize this coding-agent session output for the operator in 150 words or fewer: outcome (done/blocked/failed), key findings or changes, and open items. Output plain text, no markdown headers.";

/// Total time one sweep may spend summarizing, across every session that exited
/// in it. The daemon kills a fast-tier task at 30 seconds and drops the future;
/// the exit is claimed before the summarizer runs, so the next sweep will not
/// list that session again and an overrun loses its completion message for
/// good. This budget is what keeps the sweep inside the task timeout no matter
/// how many sessions ended at once.
const SUMMARY_SWEEP_BUDGET: Duration = Duration::from_secs(20);

/// Time the hosts get per sweep, on top of the summary budget. The task timeout
/// is 30 seconds, so this and `SUMMARY_SWEEP_BUDGET` must stay under it
/// together. A laptop that is asleep costs at most this much, not the ssh
/// connect timeout per session.
const HOST_SWEEP_BUDGET: Duration = Duration::from_secs(6);

/// A host call always gets at least this, so an almost spent budget still lets
/// a healthy local Herdr answer.
const MIN_HOST_CALL: Duration = Duration::from_secs(1);

/// Below this there is no point starting another summary: deliver the raw
/// excerpt, which costs nothing, and let the remaining exits through.
const MIN_SUMMARY_BUDGET: Duration = Duration::from_secs(3);

type SummaryFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;

/// The exit summarizer, boxed so tests can drive the supervisor with a stub
/// instead of an LLM. `None` means deliver the raw excerpt.
pub type Summarizer = Arc<dyn Fn(String) -> SummaryFuture + Send + Sync>;

/// Time a dismissal's re-read and key press get on the host.
const DISMISS_HOST_TIMEOUT: Duration = Duration::from_secs(3);

/// Screen lines quoted in a blocked-agent alert.
const BLOCKED_EXCERPT_LINES: usize = 15;

/// Resolves a registry host name to a Herdr host. Tests supply fakes.
type HostResolver = Arc<dyn Fn(&str) -> Result<Host> + Send + Sync>;

/// What the hosts said this sweep: who is alive, and each live session's screen.
struct HostPhase {
    polled: hq_tools::harness_session::HostPoll,
    screens: HashMap<String, String>,
}

/// Every host call of a sweep, under one deadline. Blocking (ssh, subprocesses),
/// so it runs on the blocking pool, and it finishes inside `budget` even when a
/// host never answers; sessions it could not reach are treated as unknown.
fn gather_hosts(
    rows: &[registry::HarnessSessionRow],
    resolve: &HostResolver,
    budget: Duration,
) -> HostPhase {
    let deadline = Instant::now() + budget;
    // A restarted built-in host holds agents until HQ supplies their env, and
    // an agent it does not list yet would be taken for gone below.
    let mut hosts: Vec<&str> = rows.iter().map(|r| r.host.as_str()).collect();
    hosts.sort_unstable();
    hosts.dedup();
    for name in hosts {
        if let Ok(host) = (resolve.as_ref())(name) {
            hq_tools::harness_session::resume_awaiting(rows, &host);
        }
    }
    let within_budget = move |host: Host| {
        host.with_command_timeout_dyn(
            deadline
                .saturating_duration_since(Instant::now())
                .max(MIN_HOST_CALL),
        )
    };
    let polled = poll_hosts_with(rows, |name| (resolve.as_ref())(name).map(within_budget));

    let mut screens = HashMap::new();
    for row in rows {
        if !matches!(liveness(&polled, row), Liveness::Alive(_)) || Instant::now() >= deadline {
            continue;
        }
        let read = (resolve.as_ref())(&row.host)
            .map(within_budget)
            .and_then(|host| Ok(host.read(&row.agent_name, SNAPSHOT_LINES)?));
        match read {
            Ok(raw) => {
                screens.insert(row.id.clone(), clean_pty_text(&raw));
            }
            Err(e) => {
                tracing::warn!(session = %row.id, error = %e, "session-supervisor: screen read failed");
            }
        }
    }
    HostPhase { polled, screens }
}


/// Cleaned final output for a session: the newest stored snapshot.
fn final_output(db: &Arc<Database>, session_id: &str) -> Option<String> {
    let id = session_id.to_string();
    db.with_conn(move |c| registry::last_snapshot(c, &id))
        .ok()
        .flatten()
        .map(|raw| clean_pty_text(&raw))
        .filter(|text| !text.trim().is_empty())
}

/// Summarizer backed by the shared router. The `notification` alias is a scored
/// pool with failover, and it errors rather than hangs when nothing is
/// reachable, which is exactly what the raw-excerpt fallback needs.
fn llm_summarizer(timeout_secs: u64) -> Summarizer {
    Arc::new(move |text: String| -> SummaryFuture {
        Box::pin(async move {
            use hq_llm::LlmProvider;
            let router = hq_llm::router::LlmRouter::from_env();
            let request = hq_llm::provider::ChatRequest {
                model: "notification".to_string(),
                messages: vec![ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: format!("{SUMMARY_PROMPT}\n\n{text}"),
                    tool_calls: vec![],
                    tool_call_id: None,
                    reasoning_content: None,
                }],
                ..Default::default()
            };
            let response =
                tokio::time::timeout(Duration::from_secs(timeout_secs), router.chat(&request))
                    .await
                    .map_err(|_| anyhow!("summary timed out after {timeout_secs}s"))??;
            let summary = response.message.content.trim().to_string();
            if summary.is_empty() {
                bail!("summary came back empty");
            }
            Ok(summary)
        })
    })
}

pub async fn run_session_supervisor(
    vault_path: &Path,
    db: &Database,
    config: &HqConfig,
) -> Result<()> {
    let summarizer = summarizer_for(config);
    let resolve: HostResolver = Arc::new(|name| hq_tools::herdr::host(Some(name)));
    supervise(
        vault_path,
        db,
        summarizer.as_ref(),
        resolve,
        HOST_SWEEP_BUDGET,
    )
    .await
}

fn summarizer_for(config: &HqConfig) -> Option<Summarizer> {
    config
        .relay
        .summarize_session_exits
        .then(|| llm_summarizer(config.relay.session_exit_summary_timeout_secs))
}

/// Store a live session's screen, keep its resume token current, and alert
/// once when it blocks on a dialog.
fn sweep_alive(
    db: &Database,
    vault_path: &Path,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    dismissed: bool,
) {
    let db_arc = Arc::new(db.clone());
    let (id, status) = (row.id.clone(), agent.status.as_str());
    if let Err(e) = db.with_conn(move |c| registry::set_seen(c, &id, status)) {
        tracing::warn!(session = %row.id, error = %e, "session-supervisor: status store failed");
    }
    if !screen.trim().is_empty() {
        let _ = hq_tools::harness_session::harvest_resume_token(&db_arc, &row.id, screen);
        let (id, snapshot) = (row.id.clone(), last_lines(screen, SNAPSHOT_LINES));
        if let Err(e) = db.with_conn(move |c| registry::set_last_snapshot(c, &id, &snapshot)) {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: snapshot store failed");
        }
    }
    match agent.status {
        // Only the survey's own block is silenced. A finished turn always alerts: it carries
        // the driver's wake, and a state change after the keypress must not lose it.
        AgentStatus::Blocked if dismissed => {}
        AgentStatus::Blocked => alert_blocked(db, row, agent, screen),
        AgentStatus::Done => alert_finished(db, vault_path, row, agent, screen),
        _ => {}
    }
}

/// Answer a known harmless prompt (Claude Code's feedback survey) in a Drive-on session with a
/// fixed key. Observe-only sessions are never typed into. True when a key was sent, so the
/// caller does not alert about a prompt that is already gone.
async fn dismiss_survey(
    db: &Database,
    row: &registry::HarnessSessionRow,
    screen: &str,
    working: bool,
    resolve: &HostResolver,
) -> bool {
    use hq_tools::harness_session::dismiss::{Outcome, PressError, dismiss_known_prompt};
    if working || !row.drive || row.owner_thread.is_none() || screen.is_empty() {
        return false;
    }
    let (db2, row2, screen2, resolve2) =
        (db.clone(), row.clone(), screen.to_string(), resolve.clone());
    let outcome = tokio::task::spawn_blocking(move || {
        let host =
            (resolve2.as_ref())(&row2.host).map(|h| h.with_command_timeout_dyn(DISMISS_HOST_TIMEOUT));
        let (reread_host, press_host) = (host.as_ref().ok().cloned(), host.as_ref().ok().cloned());
        let target = row2.agent_name.clone();
        let reread_target = target.clone();
        dismiss_known_prompt(
            &db2,
            &row2,
            &screen2,
            working,
            move || {
                let raw = reread_host?.read(&reread_target, SNAPSHOT_LINES).ok()?;
                Some(clean_pty_text(&raw))
            },
            move |keys| {
                let host = press_host.ok_or(PressError::NotSent)?;
                host.send_keys(&target, keys).map_err(|e| {
                    // An answer from Herdr means it refused the keys; silence or a lost
                    // connection may still have delivered them.
                    if e.is_unreachable() { PressError::MaybeSent } else { PressError::NotSent }
                })
            },
        )
    })
    .await
    .unwrap_or(Outcome::Untouched);
    match outcome {
        Outcome::Dismissed { first } => {
            tracing::info!(session = %row.id, "session-supervisor: dismissed the feedback survey");
            if first {
                record_on_task(db, row, Event::PromptDismissed);
            }
            true
        }
        Outcome::CapReached => {
            notify_dismiss_cap(db, row, "showed the same known prompt more than the allowed number of times");
            false
        }
        Outcome::Stuck => {
            notify_dismiss_cap(db, row, "kept showing a known prompt that did not close after the key was sent");
            false
        }
        Outcome::Failed | Outcome::Untouched => false,
    }
}

/// Keeps a session resumable into its own conversation: stores the id the
/// agent's hooks reported and, when it is new, gives the host the matching
/// restart command. Best effort; the next sweep tries again.
async fn keep_resume_current(
    db: &Database,
    vault_path: &Path,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    resolve: &HostResolver,
) {
    let db_arc = Arc::new(db.clone());
    match hq_tools::harness_session::record_agent_session_id(&db_arc, row, agent) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: conversation id store failed");
            return;
        }
    }
    let Ok(host) = (resolve.as_ref())(&row.host) else {
        return;
    };
    let (vault, row_c, agent_c) = (vault_path.to_path_buf(), row.clone(), agent.clone());
    let refreshed = tokio::task::spawn_blocking(move || {
        hq_tools::harness_session::refresh_restart_command(&vault, &host, &row_c, &agent_c)
    })
    .await;
    match refreshed {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: restart command not updated");
        }
        Err(e) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: restart command task failed");
        }
    }
}

async fn supervise(
    vault_path: &Path,
    db: &Database,
    summarizer: Option<&Summarizer>,
    resolve: HostResolver,
    host_budget: Duration,
) -> Result<()> {
    let db_arc = Arc::new(db.clone());
    let running = db.with_conn(|c| registry::list(c, Some(registry::STATUS_RUNNING), 100))?;
    let (rows, resolver) = (running.clone(), resolve.clone());
    let phase =
        tokio::task::spawn_blocking(move || gather_hosts(&rows, &resolver, host_budget)).await?;
    // Started after the host phase so a slow host cannot spend the summaries' time.
    let summary_deadline = Instant::now() + SUMMARY_SWEEP_BUDGET;

    for row in running {
        match liveness(&phase.polled, &row) {
            Liveness::HostUnreachable(detail) => {
                tracing::debug!(session = %row.id, host = %row.host, %detail, "session-supervisor: host unreachable, session left as is");
                continue;
            }
            Liveness::Alive(agent) => {
                let screen = phase.screens.get(&row.id).map_or("", String::as_str);
                let dismissed = dismiss_survey(
                    db,
                    &row,
                    screen,
                    matches!(agent.status, AgentStatus::Working),
                    &resolve,
                )
                .await;
                sweep_alive(db, vault_path, &row, &agent, screen, dismissed);
                keep_resume_current(db, vault_path, &row, &agent, &resolve).await;
                continue;
            }
            Liveness::Gone => {}
        }

        // Claim the transition inside the UPDATE. Two overlapping sweeps would
        // otherwise both see `running` and both notify.
        let id = row.id.clone();
        if !db.with_conn(move |c| registry::set_status_exited_if_running(c, &id))? {
            continue;
        }
        let token = row.resume_token.clone();
        tracing::info!(session = %row.id, harness = %row.harness, "session-supervisor: marked exited");
        // Before the summarizer: the daemon may kill this sweep at its timeout,
        // and the claim above means no later sweep would record the exit.
        let link = record_on_task(db, &row, Event::Exited);
        if hand_to_chat(db, &row, WAKE_EXITED) {
            continue;
        }
        let task_line = task_line(link.as_ref());

        let label = session_label(&row);
        let item = ValueItem::new(
            "session-supervisor",
            ValueKind::Fyi,
            format!("Harness session '{label}' ended"),
            format!(
                "Session {} exited.{} `harness_session_logs` shows its last screen; `harness_session_resume` continues it.",
                row.id,
                if token.is_some() {
                    " Resume token saved."
                } else {
                    ""
                }
            ),
        )
        .with_dedup_key(format!("session-exit-{}", row.id));
        let _ = hq_db::value_items::emit(db, &item);

        let output = final_output(&db_arc, &row.id);
        // Two limits, different jobs: the summarizer caps a single call at the
        // configured timeout, and what is left of the sweep budget caps how
        // long this exit may hold up the ones behind it.
        let budget_left = summary_deadline.saturating_duration_since(Instant::now());
        let summary = match (summarizer, &output) {
            (Some(summarize), Some(text)) if budget_left >= MIN_SUMMARY_BUDGET => {
                let call = (**summarize)(tail_excerpt(text, MAX_TAIL_BYTES));
                match tokio::time::timeout(budget_left, call).await {
                    Ok(Ok(summary)) => Some(summary),
                    Ok(Err(e)) => {
                        tracing::warn!(session = %row.id, error = %e, "session-supervisor: summary failed, sending raw output");
                        None
                    }
                    Err(_) => {
                        tracing::warn!(session = %row.id, "session-supervisor: sweep summary budget spent, sending raw output");
                        None
                    }
                }
            }
            _ => None,
        };
        let output_block = match (summary, output) {
            (Some(summary), _) => summary,
            (None, Some(text)) => format!(
                "Final output:\n{OUTPUT_OPEN}\n{}\n{OUTPUT_CLOSE}",
                tail_excerpt(&text, MAX_TAIL_BYTES)
            ),
            (None, None) => "Final output: no output was captured for this session.".to_string(),
        };

        let subject = format!("Harness session '{label}' finished");
        let body = format!(
            "{output_block}\n\nSession `{}` ({}) exited. Resume token {}.\nContinue it with `harness_session_resume` on session `{}`.{task_line}",
            row.id,
            row.harness,
            if token.is_some() {
                "saved"
            } else {
                "not found"
            },
            row.id
        );
        post_relay_nudge(vault_path, &row.id, &subject, &body, exit_interrupts(link.as_ref()));
    }

    Ok(())
}

mod alerts;
mod text;
#[cfg(test)]
mod tests;

use alerts::*;
use text::*;
