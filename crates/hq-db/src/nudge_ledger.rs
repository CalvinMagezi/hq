//! A record of every candidate nudge and what became of it, so a quiet day can be told from a
//! broken one and a new rule can run in shadow first. Counts and ids only, in the style of
//! `offer_log`: never a message, a title or a note. Adoption data: it never leaves the device
//! (`docs/security/ADOPTION_DATA.md`).

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::adoption::DAY_SECS;

/// Days the counts cover.
pub const WINDOW_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Shown,
    Withheld,
    Expired,
    Answered,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Shown => "shown",
            Outcome::Withheld => "withheld",
            Outcome::Expired => "expired",
            Outcome::Answered => "answered",
        }
    }
}

/// Records one candidate. `rule` names why a candidate was withheld; empty otherwise.
pub fn record(
    conn: &Connection,
    source: &str,
    outcome: Outcome,
    rule: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO nudge_ledger (source, outcome, rule, at) VALUES (?1, ?2, ?3, ?4)",
        params![source, outcome.as_str(), rule, now],
    )?;
    Ok(())
}

/// Counts by outcome over the last [`WINDOW_DAYS`], only for outcomes that happened.
pub fn counts(conn: &Connection, now: i64) -> Result<BTreeMap<String, usize>> {
    let from = now - WINDOW_DAYS * DAY_SECS;
    let rows = conn
        .prepare("SELECT outcome, COUNT(*) FROM nudge_ledger WHERE at >= ?1 GROUP BY outcome")?
        .query_map(params![from], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(k, n)| (k, usize::try_from(n).unwrap_or(0)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::migrations::run(&c).unwrap();
        c
    }

    #[test]
    fn counts_group_by_outcome_and_leave_out_old_rows() {
        let c = conn();
        let now = 10_000_000;
        record(&c, "daily_brief", Outcome::Shown, "", now).unwrap();
        record(&c, "daily_brief", Outcome::Shown, "", now - 5).unwrap();
        record(&c, "pin_reminder", Outcome::Withheld, "one_a_day", now).unwrap();
        record(
            &c,
            "daily_brief",
            Outcome::Shown,
            "",
            now - (WINDOW_DAYS + 1) * DAY_SECS,
        )
        .unwrap();
        let counts = counts(&c, now).unwrap();
        assert_eq!(counts.get("shown"), Some(&2));
        assert_eq!(counts.get("withheld"), Some(&1));
        assert_eq!(counts.get("expired"), None);
    }

    #[test]
    fn an_unknown_outcome_cannot_be_stored() {
        let c = conn();
        assert!(
            c.execute(
                "INSERT INTO nudge_ledger (source, outcome, at) VALUES ('x','sent',1)",
                []
            )
            .is_err()
        );
    }
}
