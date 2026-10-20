//! Pushes a `task:sync` event to web clients whenever the tasks table changes.
//!
//! `tasks_api.rs` broadcasts its own writes, but a task written by an agent tool
//! (a relay session or `/mcp`) has no REST request to broadcast from. Those run
//! in this process, so `hq_db::tasks::on_change` wakes the watcher at once. A
//! slow poll remains for writers in other processes, such as the stdio `hq mcp`.

use crate::WsState;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const OTHER_PROCESS_POLL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TasksSnapshot {
    latest_updated_at: String,
    count: i64,
    // Leases and comments change without touching a task row, so what they look like is part of
    // the fingerprint: an agent's comment over MCP then reaches an open drawer.
    leases: String,
    comments: i64,
    // Who wrote which note, and what was decided about it, feed a task's activity view.
    provenance: String,
    // Shelving changes what a list shows without touching `updated_at`.
    shelf: String,
}

fn take_tasks_snapshot(db: &hq_db::Database) -> TasksSnapshot {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(updated_at), ''), COUNT(*), \
                    (SELECT COUNT(*) || '/' || COALESCE(MAX(started_at), '') || '/' || COALESCE(MAX(ended_at), '') \
                       FROM task_work_sessions), \
                    (SELECT COALESCE(MAX(id), 0) FROM task_comments), \
                    (SELECT COALESCE(MAX(id), 0) || '/' || COUNT(*) || '/' || COALESCE(SUM(recorded_late), 0) \
                       FROM note_provenance) || '/' || \
                    (SELECT COALESCE(MAX(id), 0) FROM note_review_events), \
                    (SELECT COUNT(shelved_at) || '/' || COALESCE(MAX(shelved_at), '') FROM tasks) || '/' || \
                    (SELECT COUNT(shelved_at) || '/' || COALESCE(MAX(shelved_at), '') FROM initiatives) \
             FROM tasks",
            [],
            |row| {
                Ok(TasksSnapshot {
                    latest_updated_at: row.get(0)?,
                    count: row.get(1)?,
                    leases: row.get(2)?,
                    comments: row.get(3)?,
                    provenance: row.get(4)?,
                    shelf: row.get(5)?,
                })
            },
        )
        .map_err(anyhow::Error::from)
    })
    .unwrap_or_default()
}

pub(crate) fn spawn_tasks_watcher(state: Arc<WsState>) {
    let wake = Arc::new(Notify::new());
    let hook_wake = wake.clone();
    hq_db::tasks::on_change(move || hook_wake.notify_one());
    tokio::spawn(async move {
        let mut last = take_tasks_snapshot(&state.db);
        loop {
            let _ = tokio::time::timeout(OTHER_PROCESS_POLL, wake.notified()).await;
            let current = take_tasks_snapshot(&state.db);
            if current == last {
                continue;
            }
            last = current.clone();
            state.broadcast(
                &serde_json::json!({
                    "type": "task:sync",
                    "count": current.count,
                })
                .to_string(),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_in_process_write_is_broadcast_without_waiting_for_the_poll() {
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let mut rx = state.tx.subscribe();
        spawn_tasks_watcher(state.clone());
        tokio::task::yield_now().await;

        state
            .db
            .with_conn(|c| {
                hq_db::tasks::create_initiative(
                    c, "in-w", "personal", None, "Watch", "watch", "WATCH",
                )?;
                hq_db::tasks::create_task(
                    c,
                    "tk-w",
                    "in-w",
                    &hq_db::tasks::NewTask {
                        title: "Task",
                        created_by: "test",
                        ..Default::default()
                    },
                )
            })
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("task:sync should arrive well before the 30s fallback poll")
            .unwrap();
        assert!(event.contains("task:sync"), "{event}");
    }

    #[tokio::test]
    async fn shelving_wakes_web_clients_though_it_leaves_updated_at_alone() {
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        state
            .db
            .with_conn(|c| {
                hq_db::tasks::create_initiative(
                    c, "in-s", "personal", None, "Shelf", "shelf", "SHELF",
                )?;
                hq_db::tasks::create_task(
                    c,
                    "tk-s",
                    "in-s",
                    &hq_db::tasks::NewTask {
                        title: "Task",
                        created_by: "test",
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        let mut rx = state.tx.subscribe();
        spawn_tasks_watcher(state.clone());
        tokio::task::yield_now().await;

        state
            .db
            .with_conn(|c| hq_db::tasks::shelve_tasks(c, &["tk-s".to_string()]))
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("shelving should reach open web clients before the fallback poll")
            .unwrap();
        assert!(event.contains("task:sync"), "{event}");
    }

    #[tokio::test]
    async fn a_note_record_or_a_review_wakes_web_clients_like_any_other_task_change() {
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let mut rx = state.tx.subscribe();
        spawn_tasks_watcher(state.clone());
        tokio::task::yield_now().await;

        let record = state
            .db
            .with_conn(|c| {
                hq_db::provenance::record(
                    c,
                    "Notebooks/a.md",
                    "h1",
                    &hq_db::provenance::Attribution {
                        actor_id: Some("relay"),
                        source_kind: Some(hq_db::provenance::SourceKind::Agent),
                        session_id: Some("s-1"),
                        task_id: Some("tk-x"),
                    },
                )
            })
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a record should wake the watcher before the fallback poll")
            .unwrap();
        assert!(event.contains("task:sync"), "{event}");

        state
            .db
            .with_conn(|c| {
                hq_db::provenance::set_review_state(
                    c,
                    record.id,
                    hq_db::provenance::ReviewState::Accepted,
                    "owner",
                    None,
                )
            })
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a review should wake the watcher too")
            .unwrap();
        assert!(event.contains("task:sync"), "{event}");
    }

    #[test]
    fn snapshot_changes_when_a_lease_starts_or_ends_without_the_task_row_moving() {
        let db = hq_db::Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(
                c, "in-l", "personal", None, "Leases", "leases", "LEASE",
            )?;
            hq_db::tasks::create_task(
                c,
                "tk-l",
                "in-l",
                &hq_db::tasks::NewTask {
                    title: "T",
                    created_by: "t",
                    ..Default::default()
                },
            )
        })
        .unwrap();
        let before = take_tasks_snapshot(&db);
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO task_work_sessions (id, task_id, actor, token_hash) VALUES ('ws-l', 'tk-l', 'a', 'h')",
                [],
            )
            .map_err(anyhow::Error::from)
        })
        .unwrap();
        let started = take_tasks_snapshot(&db);
        assert_ne!(before, started);
        db.with_conn(|c| {
            c.execute("UPDATE task_work_sessions SET ended_at = datetime('now', '+1 minute'), end_reason = 'released'", [])
                .map_err(anyhow::Error::from)
        })
        .unwrap();
        assert_ne!(started, take_tasks_snapshot(&db));
    }

    #[test]
    fn snapshot_changes_when_a_comment_is_added() {
        let db = hq_db::Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-c", "personal", None, "C", "c", "CMT")?;
            hq_db::tasks::create_task(
                c,
                "tk-c",
                "in-c",
                &hq_db::tasks::NewTask {
                    title: "T",
                    created_by: "t",
                    ..Default::default()
                },
            )
        })
        .unwrap();
        let before = take_tasks_snapshot(&db);
        db.with_conn(|c| hq_db::tasks::add_comment(c, "tk-c", "agent", "progress", None))
            .unwrap();
        assert_ne!(before, take_tasks_snapshot(&db));
    }

    #[test]
    fn snapshot_changes_when_a_task_is_inserted() {
        let db = hq_db::Database::open_memory().unwrap();
        let before = take_tasks_snapshot(&db);

        db.with_conn(|c| {
            hq_db::tasks::create_initiative(
                c, "in-1", "personal", None, "Inbox", "inbox", "INBOX",
            )?;
            hq_db::tasks::create_task(
                c,
                "tk-1",
                "in-1",
                &hq_db::tasks::NewTask {
                    title: "Task",
                    created_by: "test",
                    ..Default::default()
                },
            )
        })
        .unwrap();

        let after = take_tasks_snapshot(&db);
        assert_ne!(before, after);
    }
}
