//! Tasks the person made in their own chat that fall due soon, for the daily reminders.

use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::{DIRECTION_ORIGIN, LINK_CHAT_THREAD, STATUS_COMPLETE};

/// A task with a due date, as a reminder names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTask {
    pub id: String,
    pub display_id: String,
    pub title: String,
    pub due: NaiveDate,
}

/// Open, unarchived tasks due in `[from, until]` that were created in a chat (they carry a `chat_thread`
/// origin link, which only HQ's own chat sets), soonest first, at most `limit`. Tasks agents
/// file through MCP have no such link.
pub fn due_from_chat(
    conn: &Connection,
    from: NaiveDate,
    until: NaiveDate,
    limit: usize,
) -> Result<Vec<DueTask>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.display_id, t.title, substr(t.due_date, 1, 10) AS due FROM tasks t
          WHERE t.due_date IS NOT NULL AND t.status <> ?1 AND t.archived_at IS NULL
            AND substr(t.due_date, 1, 10) BETWEEN ?2 AND ?3
            AND EXISTS (SELECT 1 FROM task_links l WHERE l.task_id = t.id
                         AND l.kind = ?4 AND l.direction = ?5)
          ORDER BY due ASC, t.display_id ASC LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            STATUS_COMPLETE,
            from.to_string(),
            until.to_string(),
            LINK_CHAT_THREAD,
            DIRECTION_ORIGIN,
            i64::try_from(limit).unwrap_or(i64::MAX)
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, display_id, title, due) = row?;
        // A malformed date is skipped rather than failing every reminder.
        if let Ok(due) = NaiveDate::parse_from_str(&due, "%Y-%m-%d") {
            out.push(DueTask {
                id,
                display_id,
                title,
                due,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn task(c: &Connection, id: &str, due: Option<&str>, status: &str, from_chat: bool) {
        c.execute(
            "INSERT OR IGNORE INTO initiatives (id, space_id, name, slug, id_prefix)
             VALUES ('i', 'personal', 'Inbox', 'inbox', 'INB')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tasks (id, initiative_id, display_id, title, status, due_date)
             VALUES (?1, 'i', ?1, ?1, ?2, ?3)",
            params![id, status, due],
        )
        .unwrap();
        let kind = if from_chat {
            LINK_CHAT_THREAD
        } else {
            "session"
        };
        c.execute(
            "INSERT INTO task_links (task_id, kind, ref, direction) VALUES (?1, ?2, 'x', ?3)",
            params![id, kind, DIRECTION_ORIGIN],
        )
        .unwrap();
    }

    #[test]
    fn only_open_chat_tasks_due_in_the_window_are_reminded() {
        let db = Database::open_memory().unwrap();
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let found = db
            .with_conn(|c| {
                task(c, "a", Some("2026-11-06"), "to_do", true);
                task(c, "b", Some("2026-11-01"), "in_progress", true);
                task(c, "c", Some("2026-11-06"), "to_do", false);
                task(c, "d", Some("2026-11-06"), STATUS_COMPLETE, true);
                task(c, "e", Some("2026-10-01"), "to_do", true);
                task(c, "f", None, "to_do", true);
                task(c, "g", Some("2026-11-30"), "to_do", true);
                task(c, "h", Some("2026-11-05"), "to_do", true);
                c.execute(
                    "UPDATE tasks SET archived_at = datetime('now') WHERE id = 'h'",
                    [],
                )?;
                due_from_chat(c, d("2026-10-29"), d("2026-11-06"), 10)
            })
            .unwrap();
        let ids: Vec<&str> = found.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
        assert_eq!(found[1].due, d("2026-11-06"));
    }
}
