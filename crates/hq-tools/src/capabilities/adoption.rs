//! The numbers that say whether HQ is becoming more useful to the person, from their chat
//! messages and their own sessions' tool calls: how soon they got a first useful result, which
//! capabilities they use, how often they had to correct HQ, and whether they keep coming back.
//!
//! These are the "before" for every later change to how HQ shows itself. Read locally, no model.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use hq_db::adoption::{
    DAY_SECS, ToolUse, UserMessage, WEEK_SECS, active_days, first_user_message_at, is_correction,
    owner_tool_use, persons_own, user_messages_since,
};
use rusqlite::Connection;

use super::Offer;

const MONTH_SECS: i64 = 28 * DAY_SECS;
/// Week four after the first message: days 21 to 28.
const WEEK_FOUR_START: i64 = 21 * DAY_SECS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptionReport {
    /// When the person first wrote to HQ; `None` means they never have.
    pub first_message_at: Option<i64>,
    /// The first capability used after that, and how many seconds after.
    pub first_value: Option<(String, i64)>,
    /// Capabilities used in the last 7 days, in catalog order, as the person reads them.
    pub used_this_week: Vec<String>,
    pub used_last_week: usize,
    /// Ready capabilities never used, in catalog order.
    pub never_used: Vec<String>,
    /// Capabilities ready on this install.
    pub ready: usize,
    /// Distinct capabilities used in the first 28 days (so far, if that is not over yet).
    pub first_month_distinct: usize,
    pub first_month_over: bool,
    pub messages_this_week: usize,
    pub corrections_this_week: usize,
    pub corrections_last_week: usize,
    pub active_days_7: usize,
    pub active_days_28: usize,
    /// Whether the person wrote in week four; `None` until week four is over.
    pub returned_week_four: Option<bool>,
}

impl AdoptionReport {
    /// The report as `hq adoption --json` and `GET /api/adoption` print it: counts and catalog
    /// wording only.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "first_message_at": self.first_message_at,
            "first_value": self.first_value.as_ref().map(|(says, secs)| serde_json::json!({"capability": says, "after_secs": secs})),
            "used_this_week": self.used_this_week,
            "used_last_week": self.used_last_week,
            "never_used": self.never_used,
            "ready": self.ready,
            "first_month_distinct": self.first_month_distinct,
            "first_month_over": self.first_month_over,
            "messages_this_week": self.messages_this_week,
            "corrections_this_week": self.corrections_this_week,
            "corrections_last_week": self.corrections_last_week,
            "active_days_7": self.active_days_7,
            "active_days_28": self.active_days_28,
            "returned_week_four": self.returned_week_four,
        })
    }
}

/// The capabilities, as the person reads them, that any of `used` belongs to, in catalog order.
fn used_capabilities(offers: &[Offer], used: &HashMap<String, ToolUse>) -> Vec<String> {
    offers
        .iter()
        .filter(|o| o.tools.iter().any(|t| used.contains_key(t)))
        .map(|o| o.says.clone())
        .collect()
}

fn count_corrections(messages: &[UserMessage], from: i64, until: i64) -> usize {
    messages
        .iter()
        .filter(|m| m.at >= from && m.at < until && is_correction(&m.text))
        .count()
}

/// The report as of `now`. `offers` is the resolved catalog (`super::resolve`).
pub fn report(conn: &Connection, now: i64, offers: &[Offer]) -> Result<AdoptionReport> {
    let Some(first) = first_user_message_at(conn)? else {
        return Ok(AdoptionReport::default());
    };
    let week_ago = now - WEEK_SECS;
    let this_week = owner_tool_use(conn, week_ago, now + 1)?;
    let last_week = owner_tool_use(conn, now - 2 * WEEK_SECS, week_ago)?;
    let ever = owner_tool_use(conn, 0, now + 1)?;
    let first_month = owner_tool_use(conn, first, first + MONTH_SECS)?;

    // The earliest call, at or after the first message, of a tool that belongs to a capability.
    let since_first = owner_tool_use(conn, first, now + 1)?;
    let first_value = offers
        .iter()
        .flat_map(|o| {
            o.tools
                .iter()
                .filter_map(|t| since_first.get(t))
                .map(move |u| (o.says.clone(), u.first_at))
        })
        .min_by_key(|(_, at)| *at)
        .map(|(says, at)| (says, (at - first).max(0)));

    let messages = persons_own(user_messages_since(
        conn,
        (now - MONTH_SECS).min(now - 2 * WEEK_SECS),
    )?);
    let returned_week_four = if now >= first + MONTH_SECS {
        let window = persons_own(user_messages_since(conn, first + WEEK_FOUR_START)?);
        Some(window.iter().any(|m| m.at < first + MONTH_SECS))
    } else {
        None
    };

    let ready: Vec<&Offer> = offers.iter().filter(|o| o.is_ready()).collect();
    let distinct: BTreeSet<String> = used_capabilities(offers, &first_month)
        .into_iter()
        .collect();
    Ok(AdoptionReport {
        first_message_at: Some(first),
        first_value,
        used_this_week: used_capabilities(offers, &this_week),
        used_last_week: used_capabilities(offers, &last_week).len(),
        never_used: ready
            .iter()
            .filter(|o| !o.tools.iter().any(|t| ever.contains_key(t)))
            .map(|o| o.says.clone())
            .collect(),
        ready: ready.len(),
        first_month_distinct: distinct.len(),
        first_month_over: now >= first + MONTH_SECS,
        messages_this_week: messages.iter().filter(|m| m.at >= week_ago).count(),
        corrections_this_week: count_corrections(&messages, week_ago, now + 1),
        corrections_last_week: count_corrections(&messages, now - 2 * WEEK_SECS, week_ago),
        active_days_7: active_days(&messages, now, 7),
        active_days_28: active_days(&messages, now, 28),
        returned_week_four,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{Install, catalog, resolve};
    use hq_db::Database;
    use hq_db::adoption::OWNER_AGENT;
    use rusqlite::params;
    use std::collections::HashSet;

    const NOW: i64 = 1_800_000_000;

    fn offers() -> Vec<Offer> {
        let install = Install {
            model: true,
            ocr: false,
            email: false,
            helpers: true,
        };
        resolve(catalog(), &install, &|_: &str| true, &HashSet::new())
    }

    fn call(c: &Connection, tool: &str, at: i64) {
        c.execute(
            "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success)
             VALUES (?1, ?2, 's', ?3, 1)",
            params![tool, OWNER_AGENT, at],
        )
        .unwrap();
    }

    fn say(c: &Connection, at: i64, text: &str) {
        c.execute(
            "INSERT OR IGNORE INTO chat_threads (thread_id, title, created_at, updated_at)
             VALUES ('t', 't', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let when = chrono::DateTime::<chrono::Utc>::from_timestamp(at, 0)
            .unwrap()
            .to_rfc3339();
        c.execute(
            "INSERT INTO chat_messages (message_id, thread_id, role, content, created_at)
             VALUES (?1, 't', 'user', ?2, ?3)",
            params![uuid::Uuid::new_v4().to_string(), text, when],
        )
        .unwrap();
    }

    #[test]
    fn a_person_who_never_wrote_has_an_empty_report() {
        let db = Database::open_memory().unwrap();
        let r = db.with_conn(|c| report(c, NOW, &offers())).unwrap();
        assert_eq!(r, AdoptionReport::default());
    }

    #[test]
    fn the_report_reads_first_value_use_corrections_and_return() {
        let db = Database::open_memory().unwrap();
        let first = NOW - 40 * DAY_SECS;
        let r = db
            .with_conn(|c| {
                say(c, first, "hello");
                // A tool call before the first message does not count as its first value.
                call(c, "vault_search", first - 100);
                call(c, "vault_write_note", first + 180);
                call(c, "task_create", first + 2 * DAY_SECS);
                say(c, first + 22 * DAY_SECS, "back in week four");
                call(c, "vault_search", NOW - 2 * DAY_SECS);
                call(c, "task_list", NOW - 10 * DAY_SECS);
                say(c, NOW - DAY_SECS, "make it shorter");
                say(c, NOW - 2 * DAY_SECS, "what needs me");
                say(c, NOW - 9 * DAY_SECS, "No, I meant the shop");
                say(c, NOW - 9 * DAY_SECS + 60, "that's wrong");
                report(c, NOW, &offers())
            })
            .unwrap();
        assert_eq!(r.first_message_at, Some(first));
        let (says, after) = r.first_value.clone().unwrap();
        assert!(says.starts_with("Keep a thought"));
        assert_eq!(after, 180);
        assert_eq!(
            r.used_this_week,
            vec!["Find something you wrote down earlier"]
        );
        assert_eq!(r.used_last_week, 1);
        assert_eq!(r.first_month_distinct, 2);
        assert!(r.first_month_over);
        assert_eq!(r.messages_this_week, 2);
        assert_eq!(r.corrections_this_week, 1);
        assert_eq!(r.corrections_last_week, 2);
        assert_eq!(r.active_days_7, 2);
        assert_eq!(r.active_days_28, 4);
        assert_eq!(r.returned_week_four, Some(true));
        assert!(!r.never_used.iter().any(|s| s.starts_with("Find something")));
        assert!(!r.never_used.iter().any(|s| s.starts_with("Read the text")));
        assert_eq!(r.ready, offers().iter().filter(|o| o.is_ready()).count());
    }

    #[test]
    fn week_four_is_unknown_until_it_is_over() {
        let db = Database::open_memory().unwrap();
        let r = db
            .with_conn(|c| {
                say(c, NOW - 10 * DAY_SECS, "hello");
                report(c, NOW, &offers())
            })
            .unwrap();
        assert_eq!(r.returned_week_four, None);
        assert!(!r.first_month_over);
        assert_eq!(r.first_value, None);
    }
}
