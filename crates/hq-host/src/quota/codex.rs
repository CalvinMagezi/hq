//! Codex: the newest session log that carries a `rate_limits` block.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, TimeZone, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow, WindowKind};
use serde_json::Value;

use super::clamp_percent;

const LANE: &str = "codex";
const MAX_FILES: usize = 20;
/// A log older than this may not describe the allowance any more.
const STALE_AFTER_SECS: i64 = 6 * 60 * 60;
/// Only the end of a log matters, and logs grow large.
const TAIL_BYTES: u64 = 4 * 1024 * 1024;
const FIVE_HOUR_MAX_MINUTES: i64 = 6 * 60;
const WEEK_MAX_MINUTES: i64 = 8 * 24 * 60;
const SESSION_DEPTH: usize = 3;

fn kind_of(minutes: i64) -> WindowKind {
    if minutes <= FIVE_HOUR_MAX_MINUTES {
        WindowKind::FiveHour
    } else if minutes <= WEEK_MAX_MINUTES {
        WindowKind::Weekly
    } else {
        WindowKind::Monthly
    }
}

fn label_of(minutes: i64) -> String {
    match kind_of(minutes) {
        WindowKind::FiveHour => format!("{} hours", (minutes + 59) / 60),
        WindowKind::Weekly => "7 days".into(),
        _ => format!("{} days", minutes / (24 * 60)),
    }
}

fn window(node: &Value, now: DateTime<Utc>) -> Option<(QuotaWindow, bool)> {
    let used = clamp_percent(node.get("used_percent")?.as_f64()?)?;
    let minutes = node.get("window_minutes").and_then(Value::as_i64)?;
    let resets_at = node
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    // A window that already reset says nothing about the new one.
    let lapsed = resets_at.is_some_and(|r| r <= now);
    let w = QuotaWindow {
        label: label_of(minutes),
        kind: kind_of(minutes),
        used_percent: Some(used),
        remaining: None,
        resets_at,
    };
    Some((w, lapsed))
}

fn rate_limits(line: &Value) -> Option<&Value> {
    [
        "/payload/rate_limits",
        "/rate_limits",
        "/payload/info/rate_limits",
    ]
    .iter()
    .filter_map(|p| line.pointer(p))
    .find(|v| v.is_object())
}

/// The last `rate_limits` block in a log's text, as a reading. `mtime` is when the log was last
/// written, which is when the numbers were true.
pub(crate) fn parse_log(text: &str, mtime: DateTime<Utc>, now: DateTime<Utc>) -> Option<LaneQuota> {
    let limits = text
        .lines()
        .rev()
        .filter(|l| l.contains("rate_limits"))
        .find_map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| rate_limits(&v).cloned())
        })?;
    let mut lapsed_any = false;
    let windows: Vec<QuotaWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| limits.get(*k))
        .filter_map(|n| window(n, now))
        .map(|(w, lapsed)| {
            lapsed_any |= lapsed;
            w
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    let old = (now - mtime).num_seconds() > STALE_AFTER_SECS;
    let stale = old || lapsed_any;
    Some(LaneQuota {
        lane: LANE.into(),
        account: limits
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        windows,
        source: QuotaSource::LocalLog,
        observed_at: mtime,
        confidence: if stale {
            Confidence::Stale
        } else {
            Confidence::Live
        },
        reason: stale.then(|| "read from an old Codex session log".to_string()),
    })
}

fn collect_logs(dir: &Path, depth: usize, out: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if depth < SESSION_DEPTH && path.is_dir() {
            collect_logs(&path, depth + 1, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_log =
            depth == SESSION_DEPTH && name.starts_with("rollout-") && name.ends_with(".jsonl");
        if is_log && let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            out.push((modified, path));
        }
    }
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn sessions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".codex")))?;
    Some(home.join("sessions"))
}

/// Reads from `root`, the folder that holds `YYYY/MM/DD/rollout-*.jsonl`.
pub(crate) fn read_from(root: &Path, now: DateTime<Utc>) -> LaneQuota {
    let mut logs = Vec::new();
    collect_logs(root, 0, &mut logs);
    logs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (modified, path) in logs.into_iter().take(MAX_FILES) {
        let Some(text) = read_tail(&path) else {
            continue;
        };
        if let Some(q) = parse_log(&text, DateTime::<Utc>::from(modified), now) {
            return q;
        }
    }
    LaneQuota::unknown(
        LANE,
        QuotaSource::LocalLog,
        "no Codex session log with a rate limit reading",
    )
}

pub(crate) fn read() -> LaneQuota {
    match sessions_dir() {
        Some(root) => read_from(&root, Utc::now()),
        None => LaneQuota::unknown(LANE, QuotaSource::LocalLog, "no home folder"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/quota/codex_rollout.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_last_block_in_a_fresh_log_is_live() {
        let q = parse_log(
            &log(),
            at("2026-11-15T10:00:00Z"),
            at("2026-11-15T10:30:00Z"),
        )
        .unwrap();
        assert_eq!(q.confidence, Confidence::Live);
        assert_eq!(q.account.as_deref(), Some("plus"));
        assert_eq!(q.windows[0].kind, WindowKind::FiveHour);
        assert_eq!(q.windows[0].used_percent, Some(31.0));
        assert_eq!(q.windows[1].kind, WindowKind::Weekly);
    }

    #[test]
    fn an_old_log_is_stale() {
        let q = parse_log(
            &log(),
            at("2026-11-15T10:00:00Z"),
            at("2026-11-15T20:00:00Z"),
        )
        .unwrap();
        assert_eq!(q.confidence, Confidence::Stale);
        assert!(q.reason.is_some());
    }

    #[test]
    fn a_log_without_limits_is_nothing() {
        assert!(
            parse_log(
                "{\"type\":\"x\"}\n",
                at("2026-11-15T10:00:00Z"),
                at("2026-11-15T10:00:00Z")
            )
            .is_none()
        );
    }

    #[test]
    fn the_newest_log_with_a_block_is_found_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026/11/15");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-a.jsonl"), log()).unwrap();
        std::fs::write(day.join("rollout-b.jsonl"), "{\"type\":\"x\"}\n").unwrap();
        let q = read_from(dir.path(), Utc::now());
        assert_eq!(q.source, QuotaSource::LocalLog);
        assert_eq!(q.windows.len(), 2);
        // Written just now but its windows reset in 2026-11: lapsed, so stale or live by clock.
        assert_ne!(q.confidence, Confidence::Unknown);
        assert_eq!(
            read_from(&dir.path().join("none"), Utc::now()).confidence,
            Confidence::Unknown
        );
    }
}
