//! The inbox tidy: moves helper requests nobody is waiting on any more out of Needs you.
//!
//! Nothing is deleted. A tidied item goes to `expired` with `expired_reason = auto_tidy:<rule>`
//! and a timestamp, shows in Done as "No longer waiting", and `restore_expired` brings it back
//! for 30 days. `hq inbox tidy` and the daily `inbox-tidy` daemon task call [`tidy`].
//!
//! Rules, in the order they are tried for an item still waiting:
//! * a new-device sign-in older than `signin_ttl_hours` (even if owner-only);
//! * a helper request (`session-supervisor`, `session_chat`) whose session ended, was stopped,
//!   is gone from the registry, or has been unreachable for over an hour, or that is older than
//!   `ttl_hours`;
//! * a helper "finished" notice older than 24 hours, which the day's fleet digest already counts.
//!
//! An owner-only helper item expires only when its session is gone or ended, never by age or by
//! silence, and an item the owner restored is never tidied a second time. A session counts as
//! unreachable only after the supervisor has swept in this process ([`mark_supervisor_swept`]):
//! right after a restart every `last_seen_at` is stale, and that says nothing about the session.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use hq_core::config::FleetInboxConfig;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Database;

/// Prefix of `expired_reason` for everything this module expires.
pub const REASON_PREFIX: &str = "auto_tidy";
/// How long a tidied item stays restorable.
pub const RESTORE_WINDOW_DAYS: i64 = 30;
/// A day's fleet digest older than this is expired; yesterday's counts are history.
const DIGEST_KEEP_DAYS: i64 = 2;
/// A helper "finished" notice older than this is folded into the digest and expired.
const FYI_FOLD_HOURS: i64 = 24;
/// A session unseen for longer than this counts as unreachable.
const UNREACHABLE_HOURS: i64 = 1;

const SIGN_IN_SOURCE: &str = "device-sign-in";
const HELPER_SOURCES: [&str; 2] = ["session-supervisor", "session_chat"];
/// Whether the session supervisor has finished a sweep since this process started.
static SUPERVISOR_SWEPT: AtomicBool = AtomicBool::new(false);

/// Called by the session supervisor after a sweep. Until then `last_seen_at` is whatever the
/// previous run left, so it is not read as proof that a session went quiet.
pub fn mark_supervisor_swept() {
    SUPERVISOR_SWEPT.store(true, Ordering::Relaxed);
}

/// Whether [`mark_supervisor_swept`] was called in this process.
pub fn supervisor_swept() -> bool {
    SUPERVISOR_SWEPT.load(Ordering::Relaxed)
}

const WAITING: &str = "state IN ('pending','routed','delivered')";

/// Why an item was tidied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    SignInTtl,
    SessionEnded,
    Unreachable,
    OlderThanTtl,
    FoldedIntoDigest,
    DigestOld,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::SignInTtl => "signin_ttl",
            Rule::SessionEnded => "session_ended",
            Rule::Unreachable => "unreachable",
            Rule::OlderThanTtl => "older_than_ttl",
            Rule::FoldedIntoDigest => "folded_into_digest",
            Rule::DigestOld => "digest_old",
        }
    }
    fn reason(self) -> String {
        format!("{REASON_PREFIX}:{}", self.as_str())
    }
}

/// How many items one rule moved (or would move), by source and kind. Counts only.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TidyGroup {
    pub source: String,
    pub kind: String,
    pub rule: Rule,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TidyReport {
    pub dry_run: bool,
    pub total: usize,
    pub groups: Vec<TidyGroup>,
}

struct Candidate {
    id: String,
    source: String,
    kind: String,
    dedup_key: Option<String>,
    created_at: DateTime<Utc>,
    owner_only: bool,
}

/// What the registry says about a session an item is about.
enum Session {
    Gone,
    Ended,
    Unreachable,
    Alive,
}

/// The session id inside a helper item's dedup key, or `None` for a key that is not about one
/// session (host notices and the like are left alone).
fn session_of(key: &str) -> Option<&str> {
    if let Some(rest) = key.strip_prefix("session-blocked-") {
        return rest.rsplit_once('-').map(|(id, _)| id);
    }
    ["session-finished-", "session-exit-", "session-dismiss-cap-"]
        .iter()
        .find_map(|p| key.strip_prefix(p))
}

fn session_state(conn: &Connection, id: &str, now: DateTime<Utc>, swept: bool) -> Result<Session> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, last_seen_at FROM harness_sessions WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((status, seen)) = row else {
        return Ok(Session::Gone);
    };
    if status != "running" {
        return Ok(Session::Ended);
    }
    if !swept {
        return Ok(Session::Alive);
    }
    let silent_for = seen
        .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| now - t.and_utc());
    Ok(match silent_for {
        Some(d) if d > Duration::hours(UNREACHABLE_HOURS) => Session::Unreachable,
        _ => Session::Alive,
    })
}

fn decide(
    conn: &Connection,
    c: &Candidate,
    now: DateTime<Utc>,
    cfg: &FleetInboxConfig,
    swept: bool,
) -> Result<Option<Rule>> {
    let age = now - c.created_at;
    if c.source == hq_core::types::FLEET_DIGEST_SOURCE {
        return Ok((age > Duration::days(DIGEST_KEEP_DAYS)).then_some(Rule::DigestOld));
    }
    if c.source == SIGN_IN_SOURCE {
        let past = c.kind == "action_needed" && age > cfg.signin_ttl();
        return Ok(past.then_some(Rule::SignInTtl));
    }
    if !HELPER_SOURCES.contains(&c.source.as_str()) {
        return Ok(None);
    }
    let Some(key) = c.dedup_key.as_deref() else {
        return Ok(None);
    };
    let Some(session) = session_of(key) else {
        return Ok(None);
    };
    if c.kind == "fyi" {
        // Without a digest nothing counts these notices, so they are not folded away.
        let fold = cfg.digest && age > Duration::hours(FYI_FOLD_HOURS);
        return Ok(fold.then_some(Rule::FoldedIntoDigest));
    }
    if c.kind != "action_needed" {
        return Ok(None);
    }
    // An exit that left its task blocked is a decision for the owner whatever became of the
    // session, so only its age retires it (and an owner-only one never expires by age).
    if key.starts_with("session-exit-") {
        let old = !c.owner_only && age > cfg.ttl();
        return Ok(old.then_some(Rule::OlderThanTtl));
    }
    match session_state(conn, session, now, swept)? {
        Session::Gone | Session::Ended => return Ok(Some(Rule::SessionEnded)),
        // An owner-only decision on a live session is never hidden.
        _ if c.owner_only => return Ok(None),
        Session::Unreachable => return Ok(Some(Rule::Unreachable)),
        Session::Alive => {}
    }
    Ok((age > cfg.ttl()).then_some(Rule::OlderThanTtl))
}

fn waiting_candidates(conn: &Connection) -> Result<Vec<Candidate>> {
    let sql = format!(
        "SELECT id, source_task, kind, dedup_key, created_at, owner_only FROM value_items
         WHERE {WAITING} AND restored_at IS NULL AND cleared_at IS NULL"
    );
    let rows = conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                source: r.get(1)?,
                kind: r.get(2)?,
                dedup_key: r.get(3)?,
                created_at: r
                    .get::<_, String>(4)?
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now()),
                owner_only: r.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Moves what has stopped waiting out of Needs you. With `dry_run` it only counts. The report
/// holds counts by source, kind and rule, never an item's text.
pub fn tidy(
    db: &Database,
    now: DateTime<Utc>,
    cfg: &FleetInboxConfig,
    dry_run: bool,
) -> Result<TidyReport> {
    tidy_with(db, now, cfg, dry_run, supervisor_swept())
}

/// [`tidy`] with the supervisor's sweep state given, for callers that know it better.
pub fn tidy_with(
    db: &Database,
    now: DateTime<Utc>,
    cfg: &FleetInboxConfig,
    dry_run: bool,
    swept: bool,
) -> Result<TidyReport> {
    db.with_conn(|conn| {
        let mut chosen = Vec::new();
        for c in waiting_candidates(conn)? {
            if let Some(rule) = decide(conn, &c, now, cfg, swept)? {
                chosen.push((c, rule));
            }
        }
        let mut counts: BTreeMap<(String, String, Rule), usize> = BTreeMap::new();
        for (c, rule) in &chosen {
            if !dry_run {
                conn.execute(
                    &format!(
                        "UPDATE value_items SET state = 'expired', expired_reason = ?2, expired_at = ?3
                         WHERE id = ?1 AND {WAITING}"
                    ),
                    params![c.id, rule.reason(), now.to_rfc3339()],
                )?;
            }
            *counts
                .entry((c.source.clone(), c.kind.clone(), *rule))
                .or_default() += 1;
        }
        Ok(TidyReport {
            dry_run,
            total: chosen.len(),
            groups: counts
                .into_iter()
                .map(|((source, kind, rule), count)| TidyGroup {
                    source,
                    kind,
                    rule,
                    count,
                })
                .collect(),
        })
    })
}

/// Brings back every item the tidy expired since `since` (never further back than
/// [`RESTORE_WINDOW_DAYS`]). A restored item is not tidied again. Returns how many came back.
pub fn restore_expired(db: &Database, since: DateTime<Utc>) -> Result<usize> {
    let now = Utc::now();
    let floor = now - Duration::days(RESTORE_WINDOW_DAYS);
    let from = since.max(floor);
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE value_items SET state = 'delivered', expired_reason = NULL, expired_at = NULL,
                    restored_at = ?2
             WHERE state = 'expired' AND expired_reason LIKE 'auto_tidy%' AND expired_at >= ?1",
            params![from.to_rfc3339(), now.to_rfc3339()],
        )?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_sessions_registry as registry;
    use hq_core::types::{ValueItem, ValueKind};

    fn cfg() -> FleetInboxConfig {
        FleetInboxConfig::default()
    }

    fn session(db: &Database, id: &str, status: &str) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "claude",
                    label: "",
                    cwd: "/tmp/example",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w",
                        pane_id: "p",
                    },
                    purpose: None,
                },
            )?;
            registry::set_seen(c, id, "blocked")?;
            if status != "running" {
                registry::set_status(c, id, status)?;
            }
            Ok(())
        })
        .unwrap();
    }

    fn put(
        db: &Database,
        source: &str,
        kind: ValueKind,
        key: &str,
        age_hours: i64,
        owner_only: bool,
    ) -> String {
        let mut item = ValueItem::new(source, kind, "t", "b")
            .with_dedup_key(key)
            .owner_only(owner_only);
        item.created_at = Utc::now() - Duration::hours(age_hours);
        crate::value_items::insert(db, &item).unwrap();
        item.id
    }

    fn state_of(db: &Database, id: &str) -> (String, Option<String>) {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT state, expired_reason FROM value_items WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap()
    }

    #[test]
    fn old_items_move_and_owner_only_ones_are_spared() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-done", "exited");
        session(&db, "hs-live", "running");
        let ended = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-done-3",
            2,
            false,
        );
        let live_new = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-live-1",
            2,
            false,
        );
        let live_old = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-live-2",
            30,
            false,
        );
        let spared_live_owner_only = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-live-9",
            24 * 40,
            true,
        );
        let old_fyi = put(
            &db,
            "session-supervisor",
            ValueKind::Fyi,
            "session-finished-hs-live",
            30,
            false,
        );
        let host = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "host-unreachable-box",
            90,
            false,
        );

        let report = tidy(&db, Utc::now(), &cfg(), false).unwrap();
        assert_eq!(report.total, 3);
        assert_eq!(
            state_of(&db, &ended).1.as_deref(),
            Some("auto_tidy:session_ended")
        );
        assert_eq!(
            state_of(&db, &live_old).1.as_deref(),
            Some("auto_tidy:older_than_ttl")
        );
        assert_eq!(
            state_of(&db, &old_fyi).1.as_deref(),
            Some("auto_tidy:folded_into_digest")
        );
        for id in [&live_new, &spared_live_owner_only, &host] {
            assert_ne!(state_of(&db, id).0, "expired");
        }
        assert!(report.groups.iter().all(|g| g.count > 0));
    }

    #[test]
    fn an_old_owner_only_item_is_tidied_and_a_stale_signin_always_is() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-done", "stopped");
        let old = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-done-1",
            24 * 8,
            true,
        );
        let signin = put(
            &db,
            SIGN_IN_SOURCE,
            ValueKind::ActionNeeded,
            "device-sign-in-abc",
            25,
            true,
        );
        let fresh_signin = put(
            &db,
            SIGN_IN_SOURCE,
            ValueKind::ActionNeeded,
            "device-sign-in-def",
            2,
            true,
        );
        tidy(&db, Utc::now(), &cfg(), false).unwrap();
        assert_eq!(state_of(&db, &old).0, "expired");
        assert_eq!(
            state_of(&db, &signin).1.as_deref(),
            Some("auto_tidy:signin_ttl")
        );
        assert_ne!(state_of(&db, &fresh_signin).0, "expired");
    }

    #[test]
    fn an_exit_that_needs_the_owner_is_kept_until_it_is_old() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-done", "exited");
        let exit = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-exit-hs-done",
            2,
            false,
        );
        tidy(&db, Utc::now(), &cfg(), false).unwrap();
        assert_ne!(state_of(&db, &exit).0, "expired");
    }

    #[test]
    fn a_dry_run_changes_nothing_but_reports_the_same_counts() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-done", "exited");
        let id = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-done-3",
            2,
            false,
        );
        let dry = tidy(&db, Utc::now(), &cfg(), true).unwrap();
        assert!(dry.dry_run);
        assert_eq!(dry.total, 1);
        assert_eq!(state_of(&db, &id), ("pending".into(), None));
        let real = tidy(&db, Utc::now(), &cfg(), false).unwrap();
        assert_eq!(real.groups, dry.groups);
    }

    #[test]
    fn restore_brings_tidied_items_back_and_they_are_not_tidied_again() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-done", "exited");
        let id = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-done-3",
            2,
            false,
        );
        tidy(&db, Utc::now(), &cfg(), false).unwrap();
        assert_eq!(state_of(&db, &id).0, "expired");
        assert_eq!(
            restore_expired(&db, Utc::now() - Duration::hours(1)).unwrap(),
            1
        );
        assert_eq!(state_of(&db, &id), ("delivered".into(), None));
        assert_eq!(tidy(&db, Utc::now(), &cfg(), false).unwrap().total, 0);
        // Nothing older than the restore window comes back.
        tidy_expired_long_ago(&db);
        assert_eq!(
            restore_expired(&db, Utc::now() - Duration::days(90)).unwrap(),
            0
        );
    }

    fn tidy_expired_long_ago(db: &Database) {
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET state='expired', expired_reason='auto_tidy:session_ended', expired_at=?1",
                [(Utc::now() - Duration::days(45)).to_rfc3339()],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn stale_last_seen(db: &Database, id: &str) {
        let id = id.to_string();
        db.with_conn(move |c| {
            c.execute(
                "UPDATE harness_sessions SET last_seen_at = datetime('now', '-3 days') WHERE id = ?1",
                [&id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_stale_last_seen_expires_nothing_until_the_supervisor_has_swept() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-live", "running");
        stale_last_seen(&db, "hs-live");
        let id = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-live-1",
            2,
            false,
        );
        // Just after a restart: last_seen_at is from before the downtime.
        let before = tidy_with(&db, Utc::now(), &cfg(), false, false).unwrap();
        assert_eq!(before.total, 0);
        assert_ne!(state_of(&db, &id).0, "expired");
        // After a sweep the silence is real.
        let after = tidy_with(&db, Utc::now(), &cfg(), true, true).unwrap();
        assert_eq!(after.total, 1, "the dry run counts what the real run moves");
        tidy_with(&db, Utc::now(), &cfg(), false, true).unwrap();
        assert_eq!(
            state_of(&db, &id).1.as_deref(),
            Some("auto_tidy:unreachable")
        );
    }

    #[test]
    fn an_owner_only_block_on_a_silent_live_session_is_never_hidden() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-live", "running");
        stale_last_seen(&db, "hs-live");
        let id = put(
            &db,
            "session-supervisor",
            ValueKind::ActionNeeded,
            "session-blocked-hs-live-1",
            24 * 30,
            true,
        );
        tidy_with(&db, Utc::now(), &cfg(), false, true).unwrap();
        assert_ne!(state_of(&db, &id).0, "expired");
    }

    #[test]
    fn without_a_digest_finished_notices_are_not_folded_away() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-live", "running");
        let id = put(
            &db,
            "session-supervisor",
            ValueKind::Fyi,
            "session-finished-hs-live",
            30,
            false,
        );
        let no_digest = FleetInboxConfig {
            digest: false,
            ..cfg()
        };
        tidy_with(&db, Utc::now(), &no_digest, false, true).unwrap();
        assert_ne!(state_of(&db, &id).0, "expired");
    }

    #[test]
    fn yesterdays_digest_expires_once_it_is_two_days_old() {
        let db = Database::open_memory().unwrap();
        let old = put(
            &db,
            hq_core::types::FLEET_DIGEST_SOURCE,
            ValueKind::Fyi,
            "fleet-digest-2026-11-01",
            24 * 3,
            false,
        );
        let fresh = put(
            &db,
            hq_core::types::FLEET_DIGEST_SOURCE,
            ValueKind::Fyi,
            "fleet-digest-2026-11-12",
            5,
            false,
        );
        tidy_with(&db, Utc::now(), &cfg(), false, false).unwrap();
        assert_eq!(
            state_of(&db, &old).1.as_deref(),
            Some("auto_tidy:digest_old")
        );
        assert_ne!(state_of(&db, &fresh).0, "expired");
    }

    #[test]
    fn migration_125_adds_the_columns_on_the_real_schema() {
        let db = Database::open_memory().unwrap();
        let id = put(&db, "agent_notify", ValueKind::Fyi, "k", 0, false);
        let cols: (i64, Option<String>, Option<String>) = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT owner_only, expired_reason, expired_at FROM value_items WHERE id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .unwrap();
        assert_eq!(cols, (0, None, None));
    }

    #[test]
    fn a_proposal_is_owner_only_by_construction() {
        let p = ValueItem::new("x", ValueKind::Proposal, "t", "b");
        assert!(p.owner_only);
        assert!(!ValueItem::new("x", ValueKind::Fyi, "t", "b").owner_only);
    }
}
