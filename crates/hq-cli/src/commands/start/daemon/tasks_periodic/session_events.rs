//! Wakes the session supervisor when the built-in host says something changed,
//! so a finished turn or a blocked dialog is noticed in seconds instead of at
//! the next once-a-minute sweep. The sweep itself is unchanged and still runs
//! on its timer as the safety net.

use super::session_supervisor::run_session_supervisor;
use hq_core::config::HqConfig;
use hq_core::types::{MailboxMessageType, ValueItem, ValueKind};
use hq_db::Database;
use hq_tools::agent_host::{self, HostEvents};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

/// How long one wait for host events lasts before asking again.
const POLL_WAIT: Duration = Duration::from_secs(25);
/// Pause after the host could not be reached, before trying again.
const RETRY_PAUSE: Duration = Duration::from_secs(10);
/// Fewest seconds between two event-driven sweeps, so a burst of events costs one.
const MIN_SWEEP_GAP: Duration = Duration::from_secs(2);

/// Whether these events are worth a sweep now: a turn finished or blocked, an
/// agent exited, or events were dropped. Spawns, removals and the agent starting
/// or working change nothing the supervisor acts on.
fn wants_sweep(events: &HostEvents) -> bool {
    events.lost
        || events.events.iter().any(|e| match e.kind.as_str() {
            "exited" => true,
            "state" => matches!(e.state.as_deref(), Some("idle" | "blocked")),
            _ => false,
        })
}

pub async fn run_session_event_loop(
    host_name: String,
    vault_path: PathBuf,
    db: Database,
    config: HqConfig,
) {
    let mut cursor: Option<u64> = None;
    loop {
        let Ok(host) = agent_host::host(Some(&host_name)) else {
            tokio::time::sleep(RETRY_PAUSE).await;
            continue;
        };
        let after = cursor;
        let polled = tokio::task::spawn_blocking(move || host.poll_events(after, POLL_WAIT)).await;
        let events = match polled {
            Ok(Ok(events)) => events,
            Ok(Err(e)) => {
                // No host running is normal when nothing uses the native host.
                debug!(host = %host_name, error = %e, "session-events: host not reachable");
                cursor = None;
                tokio::time::sleep(RETRY_PAUSE).await;
                continue;
            }
            Err(e) => {
                warn!(error = %e, "session-events: poll task failed");
                tokio::time::sleep(RETRY_PAUSE).await;
                continue;
            }
        };
        for event in events.events.iter().filter(|e| e.kind == "lockdown") {
            alert_lockdown(
                &vault_path,
                &db,
                &host_name,
                event.rule.as_deref().unwrap_or("no reason given"),
            );
        }
        let first_look = cursor.is_none();
        cursor = Some(events.last_seq);
        if first_look || !wants_sweep(&events) {
            continue;
        }
        if let Err(e) = run_session_supervisor(&vault_path, &db, &config).await {
            warn!(error = %e, "session-events: sweep failed");
        }
        tokio::time::sleep(MIN_SWEEP_GAP).await;
    }
}

/// A locked host is an emergency whoever locked it, so the owner hears at once
/// in the inbox and on the relay, interrupting whatever the relay is doing.
fn alert_lockdown(vault_path: &Path, db: &Database, host: &str, reason: &str) {
    let subject = format!("Host '{host}' is locked down");
    let body = format!(
        "No coding agent starts on '{host}' and the ones it ran were stopped (unless its start-rate breaker tripped, which only freezes new starts). Reason: {reason}. Check what started them with `host_agents` and the session logs, then lift it on that machine with `hq host unlock`."
    );
    let item = ValueItem::new(
        "agent-host",
        ValueKind::ActionNeeded,
        subject.clone(),
        body.clone(),
    )
    .with_dedup_key(format!("host-lockdown-{host}"));
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        warn!(host, error = %e, "session-events: lockdown inbox item failed");
    }
    let mut msg = hq_core::mailbox::new_message(
        "agent-host",
        "relay",
        MailboxMessageType::Nudge,
        Some(&subject),
        &body,
        None,
    );
    msg.meta.insert(
        hq_core::mailbox::META_INTERRUPT.to_string(),
        "true".to_string(),
    );
    if let Err(e) = hq_core::mailbox::send_message(vault_path, &msg) {
        warn!(host, error = %e, "session-events: lockdown relay alert failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::agent_host::HostEvent;

    fn events(list: &[(&str, Option<&str>)], lost: bool) -> HostEvents {
        HostEvents {
            events: list
                .iter()
                .enumerate()
                .map(|(i, (kind, state))| HostEvent {
                    seq: i as u64 + 1,
                    name: "a".into(),
                    kind: kind.to_string(),
                    state: state.map(str::to_string),
                    rule: None,
                    pending_seq: None,
                })
                .collect(),
            last_seq: list.len() as u64,
            lost,
        }
    }

    #[test]
    fn a_finished_turn_a_block_an_exit_or_lost_events_wake_the_supervisor() {
        assert!(wants_sweep(&events(&[("state", Some("idle"))], false)));
        assert!(wants_sweep(&events(&[("state", Some("blocked"))], false)));
        assert!(wants_sweep(&events(&[("exited", None)], false)));
        assert!(wants_sweep(&events(&[], true)));
    }

    #[test]
    fn starting_working_spawning_and_removing_do_not() {
        assert!(!wants_sweep(&events(&[("state", Some("working"))], false)));
        assert!(!wants_sweep(&events(
            &[("spawned", None), ("removed", None)],
            false
        )));
        assert!(!wants_sweep(&events(&[], false)));
    }
}
