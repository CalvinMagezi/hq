//! `hq quota`: what is left of each coding-agent lane's allowance, per host.

use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::config::HqConfig;
use hq_core::quota::Confidence;
use hq_db::Database;
use hq_tools::lane_quotas::{self, LaneView, Options, Report, WindowView, pace::PaceStatus};
use std::io::Write;

const NONE: &str = "-";
const SECS_PER_MINUTE: i64 = 60;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;

pub async fn run(
    config: &HqConfig,
    json: bool,
    refresh: bool,
    lane: Option<String>,
    host: Option<String>,
) -> Result<()> {
    let db = std::sync::Arc::new(Database::open(&config.db_path())?);
    let cfg = config.clone();
    let opts = Options {
        refresh,
        host,
        lane,
    };
    let report =
        tokio::task::spawn_blocking(move || lane_quotas::read_all(&db, &cfg, &opts)).await??;
    let mut out = std::io::stdout();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
        return Ok(());
    }
    render(&report, &mut out)
}

/// A span as `2d 3h`, `3h 20m` or `12m`.
fn span(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (
        secs / SECS_PER_DAY,
        (secs % SECS_PER_DAY) / SECS_PER_HOUR,
        (secs % SECS_PER_HOUR) / SECS_PER_MINUTE,
    );
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn used(w: &WindowView) -> String {
    if w.expired {
        return "reset".into();
    }
    let Some(u) = w.window.used_percent else {
        return w.window.remaining.clone().unwrap_or_else(|| NONE.into());
    };
    let flag = match (w.block, w.warn) {
        (true, _) => " !!",
        (_, true) => " !",
        _ => "",
    };
    format!("{u:.0}%{flag}")
}

fn resets(w: &WindowView, now: DateTime<Utc>) -> String {
    w.window.resets_at.map_or(NONE.into(), |r| {
        if r <= now {
            "passed".into()
        } else {
            format!("in {}", span((r - now).num_seconds()))
        }
    })
}

fn pace_text(w: &WindowView) -> String {
    let Some(p) = w.pace else {
        return NONE.into();
    };
    let word = match p.status {
        PaceStatus::OnPace => "on pace",
        PaceStatus::OverPace => "over pace",
        PaceStatus::OverPaceHard => "over pace, hard",
    };
    format!("{word} ({:.0}% at reset)", p.projected_percent)
}

fn confidence(l: &LaneView) -> String {
    match l.confidence {
        Confidence::Live => "live".into(),
        Confidence::Stale => format!("stale {} old", span(l.age_seconds)),
        Confidence::Unknown => "unknown".into(),
    }
}

/// One table row per window; a lane with none gets one row so it is never missing.
fn rows(report: &Report) -> Vec<[String; 8]> {
    let mut out = Vec::new();
    for l in &report.lanes {
        let base = |window: &str, used: String, resets: String, pace: String| {
            [
                l.lane.clone(),
                l.host.clone(),
                l.account.clone().unwrap_or_else(|| NONE.into()),
                window.to_string(),
                used,
                resets,
                pace,
                confidence(l),
            ]
        };
        if l.windows.is_empty() {
            out.push(base(NONE, NONE.into(), NONE.into(), NONE.into()));
        }
        for w in &l.windows {
            out.push(base(
                &w.window.label,
                used(w),
                resets(w, report.generated_at),
                pace_text(w),
            ));
        }
    }
    out
}

pub fn render(report: &Report, out: &mut dyn Write) -> Result<()> {
    if !report.enabled {
        writeln!(out, "Lane quotas are off (quota.enabled: false).")?;
        return Ok(());
    }
    let header = [
        "LANE",
        "HOST",
        "ACCOUNT",
        "WINDOW",
        "USED",
        "RESETS",
        "PACE",
        "CONFIDENCE",
    ]
    .map(String::from);
    let mut table = vec![header];
    table.extend(rows(report));
    let widths: Vec<usize> = (0..8)
        .map(|c| {
            table
                .iter()
                .map(|r| r[c].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in &table {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        writeln!(out, "{}", line.join("  ").trim_end())?;
    }
    if report.lanes.is_empty() {
        writeln!(out, "No lane reported a reading.")?;
    }
    for h in report.hosts.iter().filter(|h| !h.reachable) {
        writeln!(
            out,
            "host {}: not read ({})",
            h.host,
            h.error.as_deref().unwrap_or("unreachable")
        )?;
    }
    for l in report
        .lanes
        .iter()
        .filter(|l| l.confidence != Confidence::Live)
    {
        if let Some(why) = &l.reason {
            writeln!(out, "{}/{}: {why}", l.host, l.lane)?;
        }
    }
    let low = report
        .lanes
        .iter()
        .flat_map(|l| l.windows.iter().map(move |w| (l, w)))
        .filter(|(_, w)| w.warn)
        .count();
    if low > 0 {
        writeln!(
            out,
            "! at or above {:.0}% used, !! at or above {:.0}%",
            report.warn_percent, report.block_percent
        )?;
    }
    Ok(())
}

/// For `hq doctor`: lanes with no reading, and why.
pub async fn doctor(config: &HqConfig) {
    if !config.quota.enabled {
        return;
    }
    let Ok(db) = Database::open_existing(&config.db_path()) else {
        return;
    };
    let cfg = config.clone();
    let report =
        tokio::task::spawn_blocking(move || lane_quotas::read_all(&db, &cfg, &Options::default()))
            .await;
    let Ok(Ok(report)) = report else {
        return;
    };
    let unknown = lane_quotas::unknown_lanes(&report);
    println!("\nLane quotas");
    if unknown.is_empty() {
        println!("  every lane read here has a reading");
    }
    for (lane, why) in unknown {
        println!("  {lane}  unknown: {why}");
    }
    for h in report.hosts.iter().filter(|h| !h.reachable) {
        println!(
            "  host {}  not read: {}",
            h.host,
            h.error.as_deref().unwrap_or("unreachable")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use hq_core::quota::{LaneQuota, QuotaSource, QuotaWindow, WindowKind};

    fn report() -> Report {
        let now = Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap();
        let live = LaneQuota {
            lane: "claude".into(),
            account: Some("seat".into()),
            windows: vec![QuotaWindow {
                label: "week".into(),
                kind: WindowKind::Weekly,
                used_percent: Some(85.0),
                remaining: None,
                resets_at: Some(now + Duration::days(2)),
            }],
            source: QuotaSource::Api,
            observed_at: now,
            confidence: Confidence::Live,
            reason: None,
        };
        let unknown = LaneQuota::unknown("codex", QuotaSource::LocalLog, "no log found");
        let cfg = HqConfig::default();
        let view = |q| lane_quotas::LaneView::from_reading("laptop", q, now, &cfg.quota);
        Report {
            enabled: true,
            generated_at: now,
            warn_percent: 80.0,
            block_percent: 95.0,
            hosts: vec![lane_quotas::HostView {
                host: "laptop".into(),
                reachable: true,
                error: None,
            }],
            lanes: vec![view(live), view(unknown)],
        }
    }

    #[test]
    fn table_flags_warn_windows_and_keeps_unknown_lanes_visible() {
        let mut buf = Vec::new();
        render(&report(), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("LANE"), "{text}");
        let claude = text.lines().find(|l| l.starts_with("claude")).unwrap();
        assert!(
            claude.contains("85% !") && claude.contains("in 2d 0h"),
            "{claude}"
        );
        assert!(claude.contains("over pace"), "{claude}");
        let codex = text.lines().find(|l| l.starts_with("codex")).unwrap();
        assert!(codex.contains("unknown") && !codex.contains('%'), "{codex}");
        assert!(text.contains("laptop/codex: no log found"), "{text}");
        assert!(text.contains("! at or above 80%"), "{text}");
    }

    #[test]
    fn off_prints_one_line() {
        let mut r = report();
        r.enabled = false;
        let mut buf = Vec::new();
        render(&r, &mut buf).unwrap();
        assert!(String::from_utf8(buf).unwrap().contains("off"));
    }

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span(59), "0m");
        assert_eq!(span(3 * SECS_PER_HOUR + 20 * SECS_PER_MINUTE), "3h 20m");
        assert_eq!(span(2 * SECS_PER_DAY + 3 * SECS_PER_HOUR), "2d 3h");
    }
}
