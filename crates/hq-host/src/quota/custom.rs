//! A harness profile's own reader: a command that prints one reading as JSON.

use std::time::Duration;

use chrono::{DateTime, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow};
use serde::Deserialize;

use super::run_capture;

/// Largest stdout a custom reader may print.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 10;
/// Kept under the whole call's deadline so a slow command cannot hold the others up.
const MAX_TIMEOUT_SECS: u64 = 14;

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// One profile's reader, as the caller sends it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Spec {
    pub lane: String,
    pub command: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

/// What the command prints. Lane and source are not its to say.
#[derive(Debug, Deserialize)]
struct Printed {
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    windows: Vec<QuotaWindow>,
    #[serde(default)]
    observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    confidence: Option<Confidence>,
    #[serde(default)]
    reason: Option<String>,
}

impl Spec {
    pub fn read(&self) -> LaneQuota {
        let unknown = |why: String| LaneQuota::unknown(&self.lane, QuotaSource::Command, why);
        let timeout = Duration::from_secs(self.timeout_secs.clamp(1, MAX_TIMEOUT_SECS));
        let out = match run_capture(&self.command, None, timeout, MAX_OUTPUT_BYTES) {
            Ok(o) => o,
            Err(e) => return unknown(format!("quota command: {e}")),
        };
        let printed: Printed = match serde_json::from_str(out.stdout.trim()) {
            Ok(p) => p,
            Err(_) => return unknown("quota command did not print one JSON object".into()),
        };
        LaneQuota {
            lane: self.lane.clone(),
            account: printed.account,
            windows: printed.windows,
            source: QuotaSource::Command,
            observed_at: printed.observed_at.unwrap_or_else(Utc::now),
            confidence: printed.confidence.unwrap_or(Confidence::Live),
            reason: printed.reason,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, body: &str) -> Vec<String> {
        let path = dir.join("reader.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        vec![path.to_string_lossy().into_owned()]
    }

    fn spec(command: Vec<String>, timeout_secs: u64) -> Spec {
        Spec {
            lane: "gateway".into(),
            command,
            timeout_secs,
        }
    }

    #[test]
    fn the_printed_reading_is_taken_and_lane_and_source_are_forced() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = script(
            dir.path(),
            r#"echo '{"lane":"other","source":"api","account":"team","windows":[{"label":"month","kind":"dollars","used_percent":12.5,"remaining":"$87 left"}]}'"#,
        );
        let q = spec(cmd, 5).read();
        assert_eq!(q.lane, "gateway");
        assert_eq!(q.source, QuotaSource::Command);
        assert_eq!(q.confidence, Confidence::Live);
        assert_eq!(q.windows[0].used_percent, Some(12.5));
    }

    #[test]
    fn a_slow_command_times_out_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = script(dir.path(), "sleep 30");
        let started = std::time::Instant::now();
        let q = spec(cmd, 1).read();
        assert_eq!(q.confidence, Confidence::Unknown);
        assert!(q.reason.unwrap().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn oversize_output_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = script(dir.path(), "head -c 200000 /dev/zero | tr '\\0' 'a'");
        let q = spec(cmd, 5).read();
        assert_eq!(q.confidence, Confidence::Unknown);
        assert!(q.reason.unwrap().contains("longer than"));
    }

    #[test]
    fn garbage_and_a_missing_program_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let q = spec(script(dir.path(), "echo not json"), 5).read();
        assert_eq!(q.confidence, Confidence::Unknown);
        let q = spec(vec!["/nonexistent/reader".into()], 5).read();
        assert_eq!(q.confidence, Confidence::Unknown);
        assert_eq!(spec(Vec::new(), 5).read().confidence, Confidence::Unknown);
    }
}
