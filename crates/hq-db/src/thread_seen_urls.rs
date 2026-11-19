//! URLs a chat thread saw arrive from outside, kept across turns so a later `web_fetch` of
//! exactly one of them runs without asking. See `hq_agent::governance::seen_urls`.

use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{Connection, params};

/// URLs kept per thread; the oldest go first.
pub const MAX_PER_THREAD: usize = 2000;
/// Days a seen URL lasts.
pub const EXPIRY_DAYS: i64 = 7;

/// Every unexpired URL the thread saw.
pub fn load(conn: &Connection, thread_id: &str) -> Result<Vec<String>> {
    let floor = (Utc::now() - Duration::days(EXPIRY_DAYS)).to_rfc3339();
    let urls = conn
        .prepare("SELECT url FROM thread_seen_urls WHERE thread_id = ?1 AND seen_at >= ?2")?
        .query_map(params![thread_id, floor], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(urls)
}

/// Records `urls` for the thread, drops expired rows and keeps the newest [`MAX_PER_THREAD`].
pub fn add(conn: &Connection, thread_id: &str, urls: &[String]) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    for url in urls {
        conn.execute(
            "INSERT OR IGNORE INTO thread_seen_urls (thread_id, url, seen_at) VALUES (?1, ?2, ?3)",
            params![thread_id, url, now],
        )?;
    }
    let floor = (Utc::now() - Duration::days(EXPIRY_DAYS)).to_rfc3339();
    conn.execute(
        "DELETE FROM thread_seen_urls WHERE seen_at < ?1",
        params![floor],
    )?;
    conn.execute(
        "DELETE FROM thread_seen_urls WHERE thread_id = ?1 AND rowid NOT IN (
             SELECT rowid FROM thread_seen_urls WHERE thread_id = ?1
             ORDER BY seen_at DESC, rowid DESC LIMIT ?2)",
        params![thread_id, MAX_PER_THREAD as i64],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn urls_are_kept_per_thread_and_deduplicated() {
        let conn = conn();
        add(
            &conn,
            "a",
            &["https://x.example/".into(), "https://x.example/".into()],
        )
        .unwrap();
        add(&conn, "b", &["https://y.example/".into()]).unwrap();
        assert_eq!(
            load(&conn, "a").unwrap(),
            vec!["https://x.example/".to_string()]
        );
        assert_eq!(
            load(&conn, "b").unwrap(),
            vec!["https://y.example/".to_string()]
        );
        assert!(load(&conn, "c").unwrap().is_empty());
    }

    #[test]
    fn a_thread_keeps_at_most_the_cap() {
        let conn = conn();
        let urls: Vec<String> = (0..MAX_PER_THREAD + 5)
            .map(|i| format!("https://x.example/{i}"))
            .collect();
        add(&conn, "a", &urls).unwrap();
        assert_eq!(load(&conn, "a").unwrap().len(), MAX_PER_THREAD);
    }
}
