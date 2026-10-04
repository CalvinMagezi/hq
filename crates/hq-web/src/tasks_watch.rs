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
}

fn take_tasks_snapshot(db: &hq_db::Database) -> TasksSnapshot {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(updated_at), ''), COUNT(*) FROM tasks",
            [],
            |row| {
                Ok(TasksSnapshot {
                    latest_updated_at: row.get(0)?,
                    count: row.get(1)?,
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
                hq_db::tasks::create_initiative(c, "in-w", "personal", None, "Watch", "watch", "WATCH")?;
                hq_db::tasks::create_task(
                    c,
                    "tk-w",
                    "in-w",
                    &hq_db::tasks::NewTask { title: "Task", created_by: "test", ..Default::default() },
                )
            })
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("task:sync should arrive well before the 30s fallback poll")
            .unwrap();
        assert!(event.contains("task:sync"), "{event}");
    }

    #[test]
    fn snapshot_changes_when_a_task_is_inserted() {
        let db = hq_db::Database::open_memory().unwrap();
        let before = take_tasks_snapshot(&db);

        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Inbox", "inbox", "INBOX")?;
            hq_db::tasks::create_task(
                c,
                "tk-1",
                "in-1",
                &hq_db::tasks::NewTask { title: "Task", created_by: "test", ..Default::default() },
            )
        })
        .unwrap();

        let after = take_tasks_snapshot(&db);
        assert_ne!(before, after);
    }
}
