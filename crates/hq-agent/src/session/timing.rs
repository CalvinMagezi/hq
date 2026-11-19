//! Wall-clock bookkeeping for `SessionEvent::Timing`.

use hq_core::types::{CallTiming, TimingKind};

/// Milliseconds since the Unix epoch.
pub(super) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A started stopwatch that remembers the wall-clock time it began.
pub(super) struct Clock {
    started_at_ms: u64,
    started: std::time::Instant,
}

impl Clock {
    pub(super) fn start() -> Self {
        Self {
            started_at_ms: unix_ms(),
            started: std::time::Instant::now(),
        }
    }

    pub(super) fn finish(
        self,
        kind: TimingKind,
        name: &str,
        call_id: Option<String>,
        bytes_in: u64,
        bytes_out: u64,
    ) -> CallTiming {
        CallTiming {
            kind,
            name: name.to_string(),
            call_id,
            started_at_ms: self.started_at_ms,
            duration_ms: self.started.elapsed().as_millis() as u64,
            bytes_in,
            bytes_out,
        }
    }
}
