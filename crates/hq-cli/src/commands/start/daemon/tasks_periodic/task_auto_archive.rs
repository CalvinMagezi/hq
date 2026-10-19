use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::tasks::{SweepScope, get_task, shelve_completed};
use tracing::info;

/// Source name of the notice. Listed in `WEB_ONLY_SOURCES`: task housekeeping stays in the web app.
pub const AUTO_ARCHIVE_SOURCE: &str = "task_auto_archive";
/// Tasks named in one notice; the count still covers all of them.
const MAX_LISTED: usize = 10;

/// The notice for one sweep. The dedup key carries the date, so a restart that runs the
/// sweep twice in a day still leaves one notice.
pub fn notice_item(listed: &[(String, String)], total: usize, days: u32, day: &str) -> ValueItem {
    let mut body = String::new();
    for (display_id, title) in listed {
        body.push_str(&format!("{display_id}: {title}\n"));
    }
    if total > listed.len() {
        body.push_str(&format!("and {} more\n", total - listed.len()));
    }
    body.push_str(&format!(
        "They finished more than {days} days ago. Archiving only hides them from the web lists: agents still see them. Find them under Tasks, Archived, and bring them back with Unarchive or task_unshelve."
    ));
    let plural = if total == 1 { "" } else { "s" };
    ValueItem::new(
        AUTO_ARCHIVE_SOURCE,
        ValueKind::Fyi,
        format!("{total} completed task{plural} archived"),
        body,
    )
    .with_dedup_key(format!("task-auto-archive-{day}"))
}

/// Daily sweep: archives completed tasks older than `tasks.auto_archive_after_days`, and
/// tells the owner once. Off unless configured. It only hides tasks from the web lists, so
/// open work, a task with an open sub-task and one an agent holds are never touched.
pub async fn run_task_auto_archive(db: &Database, config: &HqConfig) -> Result<()> {
    let Some(days) = config.tasks.auto_archive_days() else {
        return Ok(());
    };
    let outcome = db.with_conn(|c| shelve_completed(c, days, &SweepScope::default(), false))?;
    if outcome.changed.is_empty() {
        return Ok(());
    }
    let listed = db.with_conn(|c| {
        let mut named = Vec::new();
        for id in outcome.changed.iter().take(MAX_LISTED) {
            if let Some(task) = get_task(c, id)? {
                named.push((task.display_id, task.title));
            }
        }
        Ok(named)
    })?;
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    hq_db::value_items::emit(db, &notice_item(&listed, outcome.changed.len(), days, &day))?;
    info!(
        count = outcome.changed.len(),
        days, "task-auto-archive: archived old completed tasks"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::tasks::{
        NewTask, STATUS_COMPLETE, TaskPatch, create_initiative, create_task, update_task,
    };

    fn finished(db: &Database, id: &str, days_ago: i64) {
        db.with_conn(|c| {
            create_task(
                c,
                id,
                "in-1",
                &NewTask {
                    title: &format!("Task {id}"),
                    created_by: "test",
                    ..Default::default()
                },
            )?;
            let done = TaskPatch {
                status: Some(STATUS_COMPLETE.to_string()),
                ..Default::default()
            };
            update_task(c, id, &done, None)?;
            c.execute(
                "UPDATE tasks SET completed_at = datetime('now', ?1) WHERE id = ?2",
                rusqlite::params![format!("-{days_ago} days"), id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn db_with_work() -> Database {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            create_initiative(
                c, "in-1", "personal", None, "Agent HQ", "agent-hq", "AGENT-HQ",
            )?;
            Ok(())
        })
        .unwrap();
        finished(&db, "old", 120);
        finished(&db, "recent", 3);
        db.with_conn(|c| {
            create_task(
                c,
                "open",
                "in-1",
                &NewTask {
                    title: "Still open",
                    created_by: "test",
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn config_with(days: Option<u32>) -> HqConfig {
        let mut config = HqConfig::default();
        config.tasks.auto_archive_after_days = days;
        config
    }

    fn archived(db: &Database) -> Vec<String> {
        let mut ids: Vec<String> = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT id FROM tasks WHERE shelved_at IS NOT NULL")?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .unwrap();
        ids.sort();
        ids
    }

    fn notices(db: &Database) -> i64 {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM value_items WHERE source_task = ?1",
                [AUTO_ARCHIVE_SOURCE],
                |r| r.get(0),
            )?)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn it_does_nothing_until_it_is_switched_on() {
        let db = db_with_work();
        for days in [None, Some(0)] {
            run_task_auto_archive(&db, &config_with(days))
                .await
                .unwrap();
        }
        assert!(archived(&db).is_empty());
        assert_eq!(notices(&db), 0);
    }

    #[tokio::test]
    async fn it_archives_only_old_finished_work_and_says_so_once() {
        let db = db_with_work();
        let config = config_with(Some(30));

        run_task_auto_archive(&db, &config).await.unwrap();

        assert_eq!(
            archived(&db),
            vec!["old"],
            "recent and open work stays shown"
        );
        assert_eq!(notices(&db), 1);
        run_task_auto_archive(&db, &config).await.unwrap();
        assert_eq!(
            notices(&db),
            1,
            "a second run finds nothing and stays quiet"
        );
    }

    #[tokio::test]
    async fn the_floor_keeps_a_typo_from_hiding_last_weeks_work() {
        let db = db_with_work();
        finished(&db, "last_week", 4);

        run_task_auto_archive(&db, &config_with(Some(1)))
            .await
            .unwrap();

        assert_eq!(
            archived(&db),
            vec!["old"],
            "1 day is raised to the 7 day floor"
        );
    }

    #[tokio::test]
    async fn work_an_agent_holds_is_never_archived() {
        let db = db_with_work();
        finished(&db, "held", 200);
        db.with_conn(|c| {
            hq_db::tasks::open_for_session(c, "held", "hs-1", "alice", "claude-code", "mac", "/w")
        })
        .unwrap();

        run_task_auto_archive(&db, &config_with(Some(30)))
            .await
            .unwrap();

        assert_eq!(archived(&db), vec!["old"]);
    }

    #[test]
    fn the_notice_names_a_few_counts_all_and_says_how_to_undo() {
        let listed: Vec<(String, String)> = (0..3)
            .map(|i| (format!("HQ-{i}"), format!("Task {i}")))
            .collect();
        let item = notice_item(&listed, 14, 30, "2026-10-19");
        assert_eq!(item.title, "14 completed tasks archived");
        assert!(item.body.contains("HQ-0: Task 0"));
        assert!(item.body.contains("and 11 more"));
        assert!(item.body.contains("agents still see them"));
        assert!(item.body.contains("task_unshelve"));
        assert_eq!(
            item.dedup_key.as_deref(),
            Some("task-auto-archive-2026-10-19")
        );
        assert_eq!(
            notice_item(&listed[..1], 1, 30, "d").title,
            "1 completed task archived"
        );
    }

    // The config ceiling must never exceed what the database sweep accepts; a build error says so.
    const _: () =
        assert!(hq_core::config::MAX_AUTO_ARCHIVE_DAYS <= hq_db::tasks::MAX_SHELVE_AGE_DAYS);

    #[test]
    fn the_notice_stays_in_the_web_app() {
        assert!(hq_daemon::value_bus::WEB_ONLY_SOURCES.contains(&AUTO_ARCHIVE_SOURCE));
    }
}
