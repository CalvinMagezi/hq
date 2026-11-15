//! One inbox item when a lane's worst window first reaches the warn line, and one more at the
//! block line, each once per reset window. The edge is stored (`lane_quota_alerts`), so a lane
//! that stays high across passes, or across a restart, does not repeat itself, and a new reset
//! time starts over.

use chrono::{DateTime, Utc};
use hq_core::quota::Confidence;
use hq_core::types::{LANE_QUOTA_SOURCE, ValueItem, ValueKind};
use hq_db::{Database, lane_quota_alerts as store, value_items};

use super::{LaneView, Report, WindowView};

const WARN: &str = "warn";
const BLOCK: &str = "block";
const SECONDS_PER_MINUTE: i64 = 60;

/// A reset time cut to the minute: a vendor that reports the same reset a few seconds apart
/// between reads is still one window.
fn to_minute(t: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp(t.timestamp() / SECONDS_PER_MINUTE * SECONDS_PER_MINUTE, 0)
        .unwrap_or(t)
}

/// The window of a lane that is highest and still open, with a reset time to key the edge on.
fn worst(lane: &LaneView) -> Option<(&WindowView, f64, DateTime<Utc>)> {
    lane.windows
        .iter()
        .filter(|w| !w.expired)
        .filter_map(|w| Some((w, w.window.used_percent?, to_minute(w.window.resets_at?))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn item(
    lane: &LaneView,
    w: &WindowView,
    used: f64,
    resets: DateTime<Utc>,
    level: &str,
) -> ValueItem {
    let pace = match w.pace {
        Some(p) => format!(
            "At this pace it reaches about {:.0}% by the reset.",
            p.projected_percent
        ),
        None => "There is not enough history to project its pace.".to_string(),
    };
    let (title, advice) = if level == BLOCK {
        (
            format!("{} lane is nearly out ({used:.0}% used)", lane.lane),
            "New sessions move to a fallback lane when the profile has one.",
        )
    } else {
        (
            format!("{} lane is running low ({used:.0}% used)", lane.lane),
            "It still works; plan the rest of the window.",
        )
    };
    let label = &w.window.label;
    let body = format!(
        "The {label} window of the {} lane on {} is at {used:.0}%. It resets at {}. {pace} {advice}",
        lane.lane,
        lane.host,
        resets.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    ValueItem::new(LANE_QUOTA_SOURCE, ValueKind::Fyi, title, body).with_dedup_key(format!(
        "lane-quota-{}-{}-{label}-{level}-{}",
        lane.host,
        lane.lane,
        resets.timestamp() / SECONDS_PER_MINUTE
    ))
}

/// Raises the items a pass is owed and returns how many. A failure to write one releases its
/// claim so the next pass tries again.
pub fn raise(db: &Database, report: &Report, now: DateTime<Utc>) -> usize {
    let mut raised = 0;
    for lane in report
        .lanes
        .iter()
        .filter(|l| l.confidence == Confidence::Live)
    {
        let Some((w, used, resets)) = worst(lane) else {
            continue;
        };
        let levels: &[&str] = match (w.block, w.warn) {
            (true, _) => &[WARN, BLOCK],
            (false, true) => &[WARN],
            _ => continue,
        };
        let key = (
            lane.host.as_str(),
            lane.lane.as_str(),
            w.window.label.as_str(),
        );
        let mut fresh = None;
        for level in levels {
            // A failed write claims nothing and sets no marker, so the next pass retries.
            let claimed = match db.with_conn(|c| store::claim(c, key, level, resets, now)) {
                Ok(new) => new,
                Err(e) => {
                    tracing::warn!(error = %e, lane = %lane.lane, "lane quota alert edge not saved");
                    break;
                }
            };
            if claimed {
                fresh = Some(*level);
            }
        }
        // Only the highest level crossed is announced; the lower one is recorded as seen.
        let Some(level) = fresh.filter(|l| Some(l) == levels.last()) else {
            continue;
        };
        let alert = item(lane, w, used, resets, level);
        // An item the owner dismissed, or that expired, still stands for this window.
        let key_text = alert.dedup_key.clone().unwrap_or_default();
        if value_items::any_with_dedup(db, &key_text).unwrap_or(false) {
            continue;
        }
        match value_items::emit(db, &alert) {
            Ok(()) => raised += 1,
            Err(e) => {
                tracing::warn!(error = %e, lane = %lane.lane, "lane quota alert failed");
                let _ = db.with_conn(|c| store::release(c, key, level, resets));
            }
        }
    }
    raised
}

/// Drops alert rows for windows that closed long ago.
pub fn prune(db: &Database, now: DateTime<Utc>) {
    let _ = db.with_conn(|c| store::prune(c, now));
}
