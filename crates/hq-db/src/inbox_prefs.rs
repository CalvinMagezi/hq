//! How the owner keeps their inbox: what they have read, what they cleared off the Done list,
//! and which topics they silenced.
//!
//! A silenced topic still lands in the inbox. It stops alerting: no push, no phone notice and no
//! count on the badge. What HQ itself asks the owner (an approval, a new sign-in, a message HQ
//! chose to send) is the topic `hq` and can never be silenced, so silencing can not hide a
//! decision that is the owner's to make.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::Database;

/// What an inbox item is about, from the source that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// HQ asking or telling the owner something itself. Never silenced.
    Hq,
    /// Helper sessions and delegated runs: prompts, finished turns, exits.
    Helpers,
    /// Tasks: reviews, verification, reminders, briefings.
    Tasks,
    /// Tips and weekly digests.
    Tips,
    /// The machine HQ runs on: disk, hosts, repairs, spending.
    Upkeep,
    /// Mail HQ triaged.
    Email,
}

/// The name of the pause, which silences every topic that can be silenced.
pub const ALL: &str = "all";

impl Topic {
    pub const SILENCEABLE: [Topic; 5] = [
        Topic::Helpers,
        Topic::Tasks,
        Topic::Tips,
        Topic::Upkeep,
        Topic::Email,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Topic::Hq => "hq",
            Topic::Helpers => "helpers",
            Topic::Tasks => "tasks",
            Topic::Tips => "tips",
            Topic::Upkeep => "upkeep",
            Topic::Email => "email",
        }
    }

    /// A topic the owner may silence, by name. `hq` is not one.
    pub fn silenceable(name: &str) -> Option<Topic> {
        Topic::SILENCEABLE.into_iter().find(|t| t.as_str() == name)
    }

    /// The topic of an item written by `source` (`value_items.source_task`). A source nobody
    /// listed is HQ's own, so a new kind of request is never silenced by accident.
    pub fn of_source(source: &str) -> Topic {
        match source {
            "session-supervisor" | "session_chat" | "harness-sessions" | "subagent_runs"
            | "background-turn" => Topic::Helpers,
            "due_reminder" => Topic::Tasks,
            s if s.starts_with("task_") => Topic::Tasks,
            "adoption_digest"
            | "collection_tip"
            | "scheduled_offer"
            | "tool_failure_digest"
            | "skill_review" => Topic::Tips,
            "disk-watchdog" | "note-repair" | "server-setup" | "budget" => Topic::Upkeep,
            "email-triage" | "email_triage" => Topic::Email,
            _ => Topic::Hq,
        }
    }
}

/// One silenced topic, or the pause (`topic` is `all`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Silence {
    pub topic: String,
    /// When it ends by itself; `None` until the owner turns it back on.
    pub until: Option<DateTime<Utc>>,
}

/// The silences in force at `now`.
pub fn silences(db: &Database, now: DateTime<Utc>) -> Result<Vec<Silence>> {
    db.with_conn(|c| {
        let rows: Vec<(String, Option<String>)> = c
            .prepare("SELECT topic, until FROM inbox_silences ORDER BY topic")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .map(|(topic, until)| Silence {
                topic,
                until: until.and_then(|u| u.parse::<DateTime<Utc>>().ok()),
            })
            .filter(|s| s.until.is_none_or(|u| u > now))
            .collect())
    })
}

/// Silences `topic` (a silenceable topic's name, or `all`) until `until`, or until it is turned
/// back on. False when the name is neither.
pub fn silence(db: &Database, topic: &str, until: Option<DateTime<Utc>>) -> Result<bool> {
    if topic != ALL && Topic::silenceable(topic).is_none() {
        return Ok(false);
    }
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO inbox_silences (topic, until, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(topic) DO UPDATE SET until = excluded.until",
            params![
                topic,
                until.map(|u| u.to_rfc3339()),
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(true)
    })
}

/// Turns a topic, or the pause, back on. True if it was silenced.
pub fn unsilence(db: &Database, topic: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.execute(
            "DELETE FROM inbox_silences WHERE topic = ?1",
            params![topic],
        )? > 0)
    })
}

/// Which sources are silent right now. Read once per pass and asked per item.
#[derive(Debug, Clone, Default)]
pub struct Silenced {
    all: bool,
    topics: Vec<String>,
}

impl Silenced {
    pub fn load(db: &Database) -> Result<Silenced> {
        let active = silences(db, Utc::now())?;
        Ok(Silenced {
            all: active.iter().any(|s| s.topic == ALL),
            topics: active.into_iter().map(|s| s.topic).collect(),
        })
    }

    /// Whether an item from `source` should stay quiet. HQ's own items never do.
    pub fn covers(&self, source: &str) -> bool {
        match Topic::of_source(source) {
            Topic::Hq => false,
            topic => self.all || self.topics.iter().any(|t| t == topic.as_str()),
        }
    }
}

/// Marks one item read. True if it was unread.
pub fn mark_read(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE value_items SET read_at = ?2 WHERE id = ?1 AND read_at IS NULL",
            params![id, Utc::now().to_rfc3339()],
        )? > 0)
    })
}

/// Marks every waiting item read. Nothing is answered or put away. Returns how many changed.
pub fn mark_all_read(db: &Database) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE value_items SET read_at = ?1
             WHERE read_at IS NULL AND state IN ('pending', 'routed', 'delivered')",
            params![Utc::now().to_rfc3339()],
        )?)
    })
}

/// Puts every waiting update away (into Done) and returns their ids, so the caller can offer
/// Undo. A request that waits for an answer is never touched: putting all away declines nothing.
pub fn archive_updates(db: &Database) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let ids: Vec<String> = c
            .prepare(
                "SELECT id FROM value_items WHERE state IN ('pending', 'routed', 'delivered')
                 AND kind NOT IN ('action_needed', 'proposal')",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &ids {
            c.execute(
                "UPDATE value_items SET state = 'dismissed', engagement = 'dismissed',
                   engaged_at = datetime('now')
                 WHERE id = ?1 AND state IN ('pending', 'routed', 'delivered')",
                params![id],
            )?;
        }
        Ok(ids)
    })
}

/// Clears the Done list: every answered, put away or expired item stops being listed. The rows
/// stay, so an approval's record and the dedup of a dismissed item are kept. Returns how many.
pub fn clear_done(db: &Database) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE value_items SET cleared_at = ?1
             WHERE cleared_at IS NULL AND state IN ('engaged', 'dismissed', 'expired')",
            params![Utc::now().to_rfc3339()],
        )?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ValueItem, ValueKind};

    fn item(db: &Database, source: &str, kind: ValueKind) -> String {
        let item = ValueItem::new(source, kind, "title", "body");
        crate::value_items::insert(db, &item).unwrap();
        item.id
    }

    fn state_of(db: &Database, id: &str) -> String {
        let id = id.to_string();
        db.with_conn(move |c| {
            Ok(
                c.query_row("SELECT state FROM value_items WHERE id = ?1", [&id], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap()
    }

    #[test]
    fn what_hq_asks_itself_can_not_be_silenced() {
        let db = Database::open_memory().unwrap();
        assert!(!silence(&db, "hq", None).unwrap());
        assert!(!silence(&db, "owner-approval", None).unwrap());
        assert!(silence(&db, ALL, None).unwrap());
        let quiet = Silenced::load(&db).unwrap();
        for source in [
            "owner-approval",
            "device-sign-in",
            "agent_notify",
            "a-source-nobody-listed",
        ] {
            assert!(
                !quiet.covers(source),
                "{source} must always reach the owner"
            );
        }
        for source in [
            "session-supervisor",
            "session_chat",
            "task_ready_for_review",
            "disk-watchdog",
        ] {
            assert!(quiet.covers(source), "{source} is paused");
        }
    }

    #[test]
    fn a_silence_covers_its_topic_until_it_ends_or_is_turned_back_on() {
        let db = Database::open_memory().unwrap();
        assert!(silence(&db, "helpers", None).unwrap());
        let quiet = Silenced::load(&db).unwrap();
        assert!(quiet.covers("session-supervisor"));
        assert!(!quiet.covers("task_briefing"));

        assert!(unsilence(&db, "helpers").unwrap());
        assert!(!Silenced::load(&db).unwrap().covers("session-supervisor"));

        let past = Utc::now() - chrono::Duration::minutes(1);
        assert!(silence(&db, "tasks", Some(past)).unwrap());
        assert!(
            !Silenced::load(&db).unwrap().covers("task_briefing"),
            "it ended"
        );
        let later = Utc::now() + chrono::Duration::hours(1);
        assert!(silence(&db, "tasks", Some(later)).unwrap());
        assert!(Silenced::load(&db).unwrap().covers("task_briefing"));
    }

    #[test]
    fn reading_puts_nothing_away_and_putting_away_declines_nothing() {
        let db = Database::open_memory().unwrap();
        let update = item(&db, "disk-watchdog", ValueKind::Fyi);
        let request = item(&db, "owner-approval", ValueKind::ActionNeeded);

        assert_eq!(mark_all_read(&db).unwrap(), 2);
        assert_eq!(mark_all_read(&db).unwrap(), 0, "already read");
        assert_eq!(state_of(&db, &update), "pending");
        assert!(!mark_read(&db, &update).unwrap());

        assert_eq!(archive_updates(&db).unwrap(), vec![update.clone()]);
        assert_eq!(state_of(&db, &update), "dismissed");
        assert_eq!(
            state_of(&db, &request),
            "pending",
            "a request waits for its answer"
        );

        assert_eq!(clear_done(&db).unwrap(), 1);
        assert_eq!(clear_done(&db).unwrap(), 0);
        assert_eq!(
            state_of(&db, &update),
            "dismissed",
            "cleared rows keep their state"
        );
    }
}
