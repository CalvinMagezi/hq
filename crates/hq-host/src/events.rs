//! A bounded log of what happened to agents, for a client that wants to hear
//! about changes without polling every agent.

use crate::detect::AgentState;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Events kept; a client that falls further behind is told `lost` and
/// re-reads the agent list.
const CAPACITY: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub name: String,
    pub kind: EventKind,
    /// The agent's state, for a `State` event.
    pub state: Option<AgentState>,
    /// The rule that decided it (`hook:<event>` for an agent's own report).
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Spawned,
    State,
    Exited,
    Removed,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Spawned => "spawned",
            EventKind::State => "state",
            EventKind::Exited => "exited",
            EventKind::Removed => "removed",
        }
    }
}

#[derive(Debug, Default)]
pub struct Poll {
    pub events: Vec<Event>,
    /// The newest sequence number, to pass as `after` next time.
    pub last_seq: u64,
    /// Some events after `after` were dropped because the log is bounded.
    pub lost: bool,
}

#[derive(Default)]
struct Inner {
    last: u64,
    events: VecDeque<Event>,
}

pub struct EventLog {
    inner: Mutex<Inner>,
    changed: Condvar,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Default for EventLog {
    fn default() -> Self {
        Self::starting_at(0)
    }
}

impl EventLog {
    /// A log whose first event is numbered `base + 1`.
    pub fn starting_at(base: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                last: base,
                events: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    /// A log numbered from the clock, so a restarted host never reuses numbers
    /// a client (or a stored alert marker) has already seen.
    pub fn from_clock() -> Self {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Self::starting_at(millis)
    }

    pub fn push(
        &self,
        name: &str,
        kind: EventKind,
        state: Option<AgentState>,
        rule: Option<String>,
    ) -> u64 {
        let mut inner = lock(&self.inner);
        inner.last += 1;
        let seq = inner.last;
        inner.events.push_back(Event {
            seq,
            name: name.to_string(),
            kind,
            state,
            rule,
        });
        while inner.events.len() > CAPACITY {
            inner.events.pop_front();
        }
        drop(inner);
        self.changed.notify_all();
        seq
    }

    pub fn last_seq(&self) -> u64 {
        lock(&self.inner).last
    }

    /// Events after `after`, waiting up to `timeout` for the first one.
    pub fn poll(&self, after: u64, timeout: Duration) -> Poll {
        let deadline = Instant::now() + timeout;
        let mut inner = lock(&self.inner);
        while inner.last <= after {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            inner = self
                .changed
                .wait_timeout(inner, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        let first = inner.events.front().map_or(inner.last + 1, |e| e.seq);
        Poll {
            events: inner.events.iter().filter(|e| e.seq > after).cloned().collect(),
            last_seq: inner.last,
            lost: inner.last > after && first > after + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const SHORT: Duration = Duration::from_millis(50);

    #[test]
    fn events_come_back_in_order_after_a_sequence_number() {
        let log = EventLog::default();
        log.push("a", EventKind::Spawned, None, None);
        log.push("a", EventKind::State, Some(AgentState::Working), None);
        let all = log.poll(0, SHORT);
        assert_eq!(all.events.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2]);
        assert!(!all.lost);
        let later = log.poll(1, SHORT);
        assert_eq!(later.events.len(), 1);
        assert_eq!(later.events[0].kind, EventKind::State);
        assert_eq!(log.poll(2, SHORT).events.len(), 0);
    }

    #[test]
    fn a_waiting_poll_wakes_when_an_event_arrives() {
        let log = Arc::new(EventLog::default());
        let writer = log.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            writer.push("a", EventKind::Exited, None, None);
        });
        let began = Instant::now();
        let got = log.poll(0, Duration::from_secs(5));
        t.join().unwrap();
        assert_eq!(got.events.len(), 1);
        assert!(began.elapsed() < Duration::from_secs(2), "woke by timeout, not by the event");
    }

    #[test]
    fn a_poll_with_nothing_new_times_out_empty() {
        let log = EventLog::default();
        let began = Instant::now();
        let got = log.poll(0, Duration::from_millis(120));
        assert!(got.events.is_empty() && !got.lost);
        assert!(began.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn a_client_that_fell_behind_is_told_events_were_lost() {
        let log = EventLog::default();
        for _ in 0..CAPACITY + 10 {
            log.push("a", EventKind::State, Some(AgentState::Idle), None);
        }
        let behind = log.poll(5, SHORT);
        assert!(behind.lost);
        assert_eq!(behind.events.len(), CAPACITY);
        let caught_up = log.poll(log.last_seq() - 3, SHORT);
        assert!(!caught_up.lost);
        assert_eq!(caught_up.events.len(), 3);
    }
}
