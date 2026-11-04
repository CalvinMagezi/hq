use anyhow::Result;
use chrono::Datelike;
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::tool_failures::{FailureDigest, MAX_SCANNED, weekly_digest};
use tracing::info;

/// Source name of the digest item, shown in the web inbox.
pub const TOOL_FAILURE_DIGEST_SOURCE: &str = "tool_failure_digest";
/// Failure groups named in one digest; the totals still cover all of them.
const MAX_LISTED: usize = 5;
const PERCENT: f64 = 100.0;

fn rate(failures: u64, calls: u64) -> f64 {
    if calls == 0 {
        return 0.0;
    }
    failures as f64 * PERCENT / calls as f64
}

/// The week's item, or none for a week with calls and no failures. A week with no calls at
/// all is reported, because that means telemetry stopped, not that nothing broke.
pub fn digest_item(d: &FailureDigest, week: &str) -> Option<ValueItem> {
    let dedup = format!("tool-failures-{week}");
    if d.calls == 0 {
        let body = "No tool calls were recorded in the last 7 days. Either HQ was idle or tool \
                    telemetry stopped; check that the daemon and the MCP gateway are running.";
        let item = ValueItem::new(
            TOOL_FAILURE_DIGEST_SOURCE,
            ValueKind::Fyi,
            "No tool calls recorded this week",
            body,
        );
        return Some(item.with_dedup_key(dedup));
    }
    if d.failures == 0 {
        return None;
    }
    let title = format!(
        "Tool failures this week: {} of {} calls ({:.1}%)",
        d.failures,
        d.calls,
        rate(d.failures, d.calls)
    );
    let item = ValueItem::new(
        TOOL_FAILURE_DIGEST_SOURCE,
        ValueKind::Fyi,
        title,
        digest_body(d),
    );
    Some(item.with_dedup_key(dedup))
}

fn digest_body(d: &FailureDigest) -> String {
    let mut body = format!(
        "Last week: {} of {} calls ({:.1}%).\n\nMost frequent:\n",
        d.last_week_failures,
        d.last_week_calls,
        rate(d.last_week_failures, d.last_week_calls)
    );
    for c in d.clusters.iter().take(MAX_LISTED) {
        body.push_str(&format!(
            "{}: {} ({} calls, {} sessions; last week {})\n",
            c.tool, c.signature, c.this_week, c.sessions, c.last_week
        ));
    }
    if d.clusters.len() > MAX_LISTED {
        body.push_str(&format!(
            "and {} smaller groups\n",
            d.clusters.len() - MAX_LISTED
        ));
    }
    if !d.llm.is_empty() {
        body.push_str("\nModel calls that failed:\n");
        for g in &d.llm {
            body.push_str(&format!(
                "{}: {} ({})\n",
                g.provider, g.error_class, g.count
            ));
        }
    }
    body.push_str(&format!("\nRead {} failure rows", d.scanned));
    if d.capped {
        body.push_str(&format!(
            " (capped at {MAX_SCANNED}; older rows were not grouped)"
        ));
    }
    body.push_str(". Nothing was changed.");
    body
}

/// Weekly digest of recurring tool failures. Reports only, from local telemetry.
pub async fn run_tool_failure_digest(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.tool_failure_digest {
        return Ok(());
    }
    let now = chrono::Utc::now();
    let digest = db.with_conn(|c| weekly_digest(c, now.timestamp()))?;
    let iso = now.iso_week();
    let week = format!("{}-W{:02}", iso.year(), iso.week());
    if let Some(item) = digest_item(&digest, &week) {
        hq_db::value_items::emit(db, &item)?;
        info!(
            failures = digest.failures,
            calls = digest.calls,
            "tool-failure-digest: reported this week's tool failures"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::tool_failures::FailureCluster;

    fn cluster(n: u64) -> FailureCluster {
        FailureCluster {
            tool: format!("tool_{n}"),
            signature: "id is required".into(),
            this_week: n,
            sessions: 1,
            last_week: 0,
        }
    }

    #[test]
    fn a_clean_week_posts_nothing_and_a_silent_week_says_telemetry_may_be_off() {
        let clean = FailureDigest {
            calls: 40,
            ..Default::default()
        };
        assert!(digest_item(&clean, "2026-W45").is_none());
        let silent = digest_item(&FailureDigest::default(), "2026-W45").unwrap();
        assert!(silent.title.contains("No tool calls"));
        assert!(silent.body.contains("telemetry"));
    }

    #[test]
    fn the_digest_names_the_top_groups_and_says_what_it_read() {
        let d = FailureDigest {
            calls: 200,
            failures: 30,
            last_week_calls: 100,
            last_week_failures: 2,
            clusters: (1..=7).rev().map(cluster).collect(),
            llm: vec![],
            scanned: 30,
            capped: true,
        };
        let item = digest_item(&d, "2026-W45").unwrap();
        assert_eq!(
            item.title,
            "Tool failures this week: 30 of 200 calls (15.0%)"
        );
        assert!(item.body.contains("Last week: 2 of 100 calls (2.0%)"));
        assert!(
            item.body
                .contains("tool_7: id is required (7 calls, 1 sessions; last week 0)")
        );
        assert!(!item.body.contains("tool_2:"), "only five are named");
        assert!(item.body.contains("and 2 smaller groups"));
        assert!(item.body.contains("capped at"));
        assert_eq!(item.dedup_key.as_deref(), Some("tool-failures-2026-W45"));
    }

    #[tokio::test]
    async fn the_switch_turns_it_off_and_one_week_gives_one_item() {
        let db = Database::open_memory().unwrap();
        let mut config = HqConfig::default();
        config.daemon.tool_failure_digest = false;
        run_tool_failure_digest(&db, &config).await.unwrap();
        let count = || -> i64 {
            db.with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM value_items WHERE source_task = ?1",
                    [TOOL_FAILURE_DIGEST_SOURCE],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
        };
        assert_eq!(count(), 0);
        config.daemon.tool_failure_digest = true;
        run_tool_failure_digest(&db, &config).await.unwrap();
        run_tool_failure_digest(&db, &config).await.unwrap();
        assert_eq!(count(), 1, "the silent-week item is posted once per week");
    }
}
