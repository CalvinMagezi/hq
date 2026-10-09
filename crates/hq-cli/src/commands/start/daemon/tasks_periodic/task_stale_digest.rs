use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::tasks::{StaleTask, stale_tasks};
use tracing::info;

/// Source name of the digest item. Listed in `WEB_ONLY_SOURCES`: task housekeeping stays in the web app.
pub const STALE_DIGEST_SOURCE: &str = "task_stale_digest";
/// Tasks named in one digest; the count still covers all of them.
const MAX_LISTED: usize = 10;
/// Most stale tasks read for one digest.
const MAX_SCANNED: usize = 500;

/// One digest item for the day, or none when nothing looks abandoned. The dedup key
/// carries the date, so a task that stays stale is mentioned once a day, not every sweep.
pub fn digest_item(stale: &[StaleTask], day: &str) -> Option<ValueItem> {
    if stale.is_empty() {
        return None;
    }
    let mut body = String::new();
    for s in stale.iter().take(MAX_LISTED) {
        body.push_str(&format!("{}: {} (quiet for {} days)\n", s.display_id, s.title, s.idle_hours / 24));
    }
    if stale.len() > MAX_LISTED {
        body.push_str(&format!("and {} more\n", stale.len() - MAX_LISTED));
    }
    body.push_str("Nothing was changed. Release, block (with a reason), close or claim each one; the task_stale tool says what each looks like.");
    let title = format!("{} in-progress tasks look abandoned", stale.len());
    Some(ValueItem::new(STALE_DIGEST_SOURCE, ValueKind::Fyi, title, body).with_dedup_key(format!("task-stale-{day}")))
}

/// Daily digest of in-progress tasks nobody holds or has touched. Reports only; it
/// never changes a task.
pub async fn run_task_stale_digest(db: &Database, config: &HqConfig) -> Result<()> {
    let hours = i64::try_from(config.tasks.stale_hours()).unwrap_or(i64::MAX);
    let ttl = i64::try_from(config.tasks.lease_ttl()).unwrap_or(i64::MAX);
    let stale = db.with_conn(|c| stale_tasks(c, hours, ttl, MAX_SCANNED))?;
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    if let Some(item) = digest_item(&stale, &day) {
        hq_db::value_items::emit(db, &item)?;
        info!(count = stale.len(), "task-stale-digest: reported abandoned in-progress tasks");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale(n: usize) -> Vec<StaleTask> {
        (0..n)
            .map(|i| StaleTask {
                task_id: format!("tk-{i}"),
                display_id: format!("FR-{i}"),
                title: format!("Task {i}"),
                initiative_id: "in-1".into(),
                last_activity_at: "2026-01-01 00:00:00".into(),
                idle_hours: 24 * (i as i64 + 4),
            })
            .collect()
    }

    #[test]
    fn nothing_stale_means_no_digest() {
        assert!(digest_item(&[], "2026-10-09").is_none());
    }

    #[test]
    fn the_digest_names_a_few_counts_all_and_says_nothing_was_changed() {
        let item = digest_item(&stale(14), "2026-10-09").unwrap();
        assert_eq!(item.title, "14 in-progress tasks look abandoned");
        assert!(item.body.contains("FR-0: Task 0 (quiet for 4 days)"));
        assert!(item.body.contains("and 4 more"));
        assert!(item.body.contains("Nothing was changed"));
        assert!(!item.body.contains("FR-10:"), "only the first ten are named");
        assert_eq!(item.dedup_key.as_deref(), Some("task-stale-2026-10-09"));
        assert_eq!(item.source_task, STALE_DIGEST_SOURCE);
    }

    #[tokio::test]
    async fn the_same_day_never_produces_a_second_item() {
        let db = Database::open_memory().unwrap();
        let item = digest_item(&stale(2), "2026-10-09").unwrap();
        hq_db::value_items::emit(&db, &item).unwrap();
        let again = digest_item(&stale(3), "2026-10-09").unwrap();
        hq_db::value_items::emit(&db, &again).unwrap();
        let count: i64 = db
            .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM value_items WHERE source_task = ?1", [STALE_DIGEST_SOURCE], |r| r.get(0))?))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn the_run_reads_the_vault_and_stays_quiet_when_all_is_well() {
        let db = Database::open_memory().unwrap();
        run_task_stale_digest(&db, &HqConfig::default()).await.unwrap();
        let count: i64 = db
            .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM value_items", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(count, 0);
    }
}
