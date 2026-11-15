//! Antigravity: `agy -p /quota` prints one tab-separated row per limit.

use chrono::{DateTime, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow, WindowKind};

use super::{HTTP_TIMEOUT, OUTPUT_CAP_BYTES, clamp_percent, run_capture};

const LANE: &str = "agy";
const PERCENT_SCALE: f64 = 100.0;
const FIELDS: usize = 4;

fn kind_of(label: &str) -> WindowKind {
    let l = label.to_lowercase();
    if l.contains("week") {
        WindowKind::Weekly
    } else if l.contains("hour") || l.contains("5h") {
        WindowKind::FiveHour
    } else {
        // A limit this reader cannot place in time is still a counted allowance.
        WindowKind::Credits
    }
}

/// Rows `group<TAB>limit label<TAB>percent left<TAB>reset time`. Rows without a percentage (a
/// header, a banner) are skipped.
pub(crate) fn parse_rows(text: &str) -> Vec<QuotaWindow> {
    text.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
            if cols.len() < FIELDS {
                return None;
            }
            let left: f64 = cols[2].trim_end_matches('%').trim().parse().ok()?;
            let reset_text = cols[3];
            let resets_at = DateTime::parse_from_rfc3339(reset_text)
                .ok()
                .map(|d| d.with_timezone(&Utc));
            let note = match resets_at {
                Some(_) => format!("{left}% left"),
                None => format!("{left}% left, resets {reset_text}"),
            };
            Some(QuotaWindow {
                label: format!("{} {}", cols[0], cols[1]).trim().to_string(),
                kind: kind_of(cols[1]),
                used_percent: clamp_percent(PERCENT_SCALE - left),
                remaining: Some(note),
                resets_at,
            })
        })
        .collect()
}

pub(crate) fn read() -> LaneQuota {
    // A scratch folder keeps agy from reading a project's files and trust prompts.
    let dir = std::env::temp_dir().join(format!("hq-quota-agy-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return LaneQuota::unknown(LANE, QuotaSource::Cli, format!("no scratch folder: {e}"));
    }
    let argv = ["agy", "-p", "/quota"].map(String::from).to_vec();
    let out = run_capture(&argv, Some(&dir), HTTP_TIMEOUT, OUTPUT_CAP_BYTES);
    let _ = std::fs::remove_dir_all(&dir);
    let out = match out {
        Ok(o) => o,
        Err(e) => return LaneQuota::unknown(LANE, QuotaSource::Cli, format!("agy: {e}")),
    };
    let windows = parse_rows(&out.stdout);
    if windows.is_empty() {
        return LaneQuota::unknown(LANE, QuotaSource::Cli, "agy /quota printed no limit rows");
    }
    LaneQuota {
        lane: LANE.into(),
        account: None,
        windows,
        source: QuotaSource::Cli,
        observed_at: Utc::now(),
        confidence: Confidence::Live,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_become_windows_and_banners_are_skipped() {
        let text = std::fs::read_to_string(format!(
            "{}/tests/fixtures/quota/agy_quota.tsv",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let w = parse_rows(&text);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].kind, WindowKind::FiveHour);
        assert_eq!(w[0].used_percent, Some(27.0));
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[1].used_percent, Some(60.0));
        assert!(w[1].remaining.as_deref().unwrap().contains("Mon 09:00"));
    }
}
