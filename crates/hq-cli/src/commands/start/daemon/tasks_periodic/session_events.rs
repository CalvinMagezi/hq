//! Wakes the session supervisor when the built-in host says something changed,
//! so a finished turn or a blocked dialog is noticed in seconds instead of at
//! the next once-a-minute sweep. The sweep itself is unchanged and still runs
//! on its timer as the safety net.

use super::session_supervisor::run_session_supervisor;
use hq_core::config::{HqConfig, NATIVE_HOST};
use hq_db::Database;
use hq_tools::herdr::{self, HostEvents};
use std::path::PathBuf;
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

pub async fn run_session_event_loop(vault_path: PathBuf, db: Database, config: HqConfig) {
    let mut cursor: Option<u64> = None;
    loop {
        let Ok(host) = herdr::host(Some(NATIVE_HOST)) else {
            tokio::time::sleep(RETRY_PAUSE).await;
            continue;
        };
        let after = cursor;
        let polled = tokio::task::spawn_blocking(move || host.poll_events(after, POLL_WAIT)).await;
        let events = match polled {
            Ok(Ok(events)) => events,
            Ok(Err(e)) => {
                // No host running is normal when nothing uses the native host.
                debug!(error = %e, "session-events: native host not reachable");
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

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::herdr::HostEvent;

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
