use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::types::{ValueItem, ValueKind, ValueState};
use rusqlite::{OptionalExtension, Row, params};

use crate::Database;

const COLS: &str = "id, source_task, kind, title, body, artifact_path, score, dedup_key, \
                    state, created_at, routed_at, delivered_at, engaged_at, expires_at, engagement";

fn ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn opt_ts(o: &Option<DateTime<Utc>>) -> Option<String> {
    o.as_ref().map(ts)
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_opt_ts(o: Option<String>) -> Option<DateTime<Utc>> {
    o.map(|s| parse_ts(&s))
}

fn row_to_item(r: &Row) -> rusqlite::Result<ValueItem> {
    let kind_s: String = r.get(2)?;
    let state_s: String = r.get(8)?;
    Ok(ValueItem {
        id: r.get(0)?,
        source_task: r.get(1)?,
        kind: ValueKind::from_str(&kind_s).unwrap_or(ValueKind::Fyi),
        title: r.get(3)?,
        body: r.get(4)?,
        artifact_path: r.get(5)?,
        score: r.get(6)?,
        dedup_key: r.get(7)?,
        state: ValueState::from_str(&state_s).unwrap_or(ValueState::Pending),
        created_at: parse_ts(&r.get::<_, String>(9)?),
        routed_at: parse_opt_ts(r.get::<_, Option<String>>(10)?),
        delivered_at: parse_opt_ts(r.get::<_, Option<String>>(11)?),
        engaged_at: parse_opt_ts(r.get::<_, Option<String>>(12)?),
        expires_at: parse_opt_ts(r.get::<_, Option<String>>(13)?),
        engagement: r.get(14)?,
    })
}

/// Raw insert. Prefer `emit` which applies dedup.
pub fn insert(db: &Database, item: &ValueItem) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO value_items (id, source_task, kind, title, body, artifact_path, score, \
             dedup_key, state, created_at, routed_at, delivered_at, engaged_at, expires_at, engagement) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                item.id,
                item.source_task,
                item.kind.as_str(),
                item.title,
                item.body,
                item.artifact_path,
                item.score,
                item.dedup_key,
                item.state.as_str(),
                ts(&item.created_at),
                opt_ts(&item.routed_at),
                opt_ts(&item.delivered_at),
                opt_ts(&item.engaged_at),
                opt_ts(&item.expires_at),
                item.engagement,
            ],
        )?;
        Ok(())
    })
}

/// Return an active (pending/routed/delivered) item sharing (kind, dedup_key), if any.
pub fn find_active_by_dedup(
    db: &Database,
    kind: ValueKind,
    dedup_key: &str,
) -> Result<Option<ValueItem>> {
    db.with_conn(|c| {
        // 'dismissed' counts as claiming the key too: an operator explicitly
        // dismissing an item is a decision that this content isn't worth
        // surfacing, and `emit`'s dedup collapse must honor that rather than
        // let the same producer re-insert an identical row next cycle (see
        // FEATURE-REQUESTS.md FR-001a's "survives a daemon cycle without
        // immediate regeneration" criterion). 'expired' stays excluded —
        // TTL expiry is not a rejection, and should still allow a fresh item
        // to surface later if the underlying condition recurs.
        let sql = format!(
            "SELECT {COLS} FROM value_items \
             WHERE kind = ?1 AND dedup_key = ?2 \
             AND state IN ('pending','routed','delivered','dismissed') LIMIT 1"
        );
        let item = c
            .query_row(&sql, params![kind.as_str(), dedup_key], row_to_item)
            .optional()?;
        Ok(item)
    })
}

/// Canonical emit: insert unless an active item with the same (kind, dedup_key)
/// already exists, in which case collapse (skip). Best-effort callers ignore the result.
pub fn emit(db: &Database, item: &ValueItem) -> Result<()> {
    if let Some(key) = &item.dedup_key
        && find_active_by_dedup(db, item.kind, key)?.is_some()
    {
        return Ok(());
    }
    insert(db, item)
}

/// Open the vault database and emit. For producers that only hold a vault path.
pub fn emit_at(vault_path: &std::path::Path, item: &ValueItem) -> Result<()> {
    let db_path = vault_path.join("_data").join("vault.db");
    let db = Database::open(&db_path)?;
    emit(&db, item)
}

pub fn list_by_state(db: &Database, state: ValueState) -> Result<Vec<ValueItem>> {
    db.with_conn(|c| {
        let sql = format!(
            "SELECT {COLS} FROM value_items WHERE state = ?1 ORDER BY score DESC, created_at ASC"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(params![state.as_str()], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn set_delivered(db: &Database, id: &str) -> Result<()> {
    db.with_conn(|c| {
        let now = ts(&Utc::now());
        c.execute(
            "UPDATE value_items SET state='delivered', routed_at=COALESCE(routed_at, ?2), delivered_at=?2 WHERE id=?1",
            params![id, now],
        )?;
        Ok(())
    })
}

/// Record engagement on a delivered item whose id starts with `token`.
/// Returns true if a row matched. `outcome` is "approved" or "dismissed".
pub fn record_engagement_by_token(db: &Database, token: &str, outcome: &str) -> Result<bool> {
    // Reached from an unauthenticated web API route (hq-web's notifications
    // endpoint takes this straight from a URL path segment) — a short or
    // wildcard-only token must never become "match every delivered row".
    if token.len() < MIN_ID_PREFIX_LEN {
        return Ok(false);
    }
    let state = if outcome == "approved" {
        "engaged"
    } else {
        "dismissed"
    };
    db.with_conn(|c| {
        let pattern = format!("{}%", escape_like(token));
        let ids = matching_ids(c, &pattern, "delivered")?;
        let n = c.execute(
            "UPDATE value_items SET state=?3, engagement=?2, engaged_at=?4 \
             WHERE id LIKE ?1 ESCAPE '\\' AND state='delivered'",
            params![pattern, outcome, state, ts(&Utc::now())],
        )?;
        if state == "engaged" {
            sign_approvals(c, &ids);
        }
        Ok(n > 0)
    })
}

/// Ids matching `pattern` whose state is in `states` (a pre-quoted list body).
fn matching_ids(c: &rusqlite::Connection, pattern: &str, states: &str) -> Result<Vec<String>> {
    // `states` is only ever one of this file's own literals.
    let sql = format!("SELECT id FROM value_items WHERE id LIKE ?1 ESCAPE '\\' AND state IN ('{states}')");
    let mut stmt = c.prepare(&sql)?;
    let ids = stmt
        .query_map(params![pattern], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Signs each approved item with the per-install key. Without a key file the
/// items stay unsigned, which every consumer of a signature treats as unapproved.
fn sign_approvals(c: &rusqlite::Connection, ids: &[String]) {
    let Ok(key) = hq_core::approval_key::load_or_create_key() else {
        tracing::warn!("approval key unavailable; approvals will not be signed");
        return;
    };
    for id in ids {
        let mac = hq_core::approval_key::hmac_hex(&key, &hq_core::approval_key::value_approval_message(id));
        let _ = c.execute("UPDATE value_items SET engagement_mac = ?2 WHERE id = ?1", params![id, mac]);
    }
}

/// True when the newest item with this dedup key was approved through a code
/// path that holds the install key (chat button, web UI, `hq queue approve`).
pub fn is_signed_approval(db: &Database, dedup_key: &str) -> Result<bool> {
    let Ok(key) = hq_core::approval_key::load_or_create_key() else {
        return Ok(false);
    };
    db.with_conn(|c| {
        let row: Option<(String, String, Option<String>)> = c
            .query_row(
                "SELECT id, state, engagement_mac FROM value_items WHERE dedup_key = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![dedup_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((id, state, Some(mac))) if state == "engaged" => hq_core::approval_key::verify_hex(
                &key,
                &hq_core::approval_key::value_approval_message(&id),
                &mac,
            ),
            _ => false,
        })
    })
}

/// State of the newest item with this dedup key in any state, e.g. whether an
/// approval request was engaged or dismissed. `None` when never emitted.
pub fn latest_state_by_dedup(db: &Database, dedup_key: &str) -> Result<Option<ValueState>> {
    db.with_conn(|c| {
        let state: Option<String> = c
            .query_row(
                "SELECT state FROM value_items WHERE dedup_key = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![dedup_key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(state.and_then(|s| ValueState::from_str(&s)))
    })
}

/// Approve one active item (pending/routed/delivered) by id or id-prefix, for
/// operators without a chat relay. Same prefix rules as [`dismiss_by_id`].
pub fn approve_by_id(db: &Database, id_or_token: &str) -> Result<bool> {
    if id_or_token.len() < MIN_ID_PREFIX_LEN {
        anyhow::bail!(
            "id/token '{id_or_token}' is too short (minimum {MIN_ID_PREFIX_LEN} characters)"
        );
    }
    db.with_conn(|c| {
        let pattern = format!("{}%", escape_like(id_or_token));
        let ids = matching_ids(c, &pattern, "pending','routed','delivered")?;
        let n = c.execute(
            "UPDATE value_items SET state='engaged', engagement='approved', engaged_at=?2 \
             WHERE id LIKE ?1 ESCAPE '\\' AND state IN ('pending','routed','delivered')",
            params![pattern, ts(&Utc::now())],
        )?;
        sign_approvals(c, &ids);
        Ok(n > 0)
    })
}

/// Expire pending/routed items past their TTL. Delivered items are kept for the ledger.
pub fn gc_expired(db: &Database) -> Result<usize> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET state='expired' \
             WHERE expires_at IS NOT NULL AND expires_at < ?1 AND state IN ('pending','routed')",
            params![ts(&Utc::now())],
        )?;
        Ok(n)
    })
}

/// List items, most-recent first, optionally filtered by state/kind. Used by
/// `hq queue list`, so there's no CLI-less path that requires raw SQL.
pub fn list_filtered(
    db: &Database,
    state: Option<ValueState>,
    kind: Option<ValueKind>,
    limit: usize,
) -> Result<Vec<ValueItem>> {
    db.with_conn(|c| {
        let mut clauses = Vec::new();
        if state.is_some() {
            clauses.push("state = ?1".to_string());
        }
        if kind.is_some() {
            clauses.push(format!("kind = ?{}", if state.is_some() { 2 } else { 1 }));
        }
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let sql =
            format!("SELECT {COLS} FROM value_items {where_clause} ORDER BY created_at DESC LIMIT ?{}",
                clauses.len() + 1);
        let mut stmt = c.prepare(&sql)?;
        let rows = match (state, kind) {
            (Some(s), Some(k)) => stmt
                .query_map(params![s.as_str(), k.as_str(), limit as i64], row_to_item)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (Some(s), None) => stmt
                .query_map(params![s.as_str(), limit as i64], row_to_item)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, Some(k)) => stmt
                .query_map(params![k.as_str(), limit as i64], row_to_item)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            (None, None) => stmt
                .query_map(params![limit as i64], row_to_item)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    })
}

/// Count of items per state (all states, including terminal ones). Used by
/// `hq queue stats`.
pub fn count_by_state(db: &Database) -> Result<Vec<(String, i64)>> {
    db.with_conn(|c| {
        let mut stmt =
            c.prepare("SELECT state, COUNT(*) FROM value_items GROUP BY state ORDER BY state")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Escape SQLite `LIKE` wildcards (`%`, `_`) in raw, externally-supplied text
/// so a prefix like `%` or `_` can't turn a targeted lookup into "match every
/// row". Pair with `LIKE ... ESCAPE '\'` at every call site — this must be
/// applied everywhere an id/token prefix reaches a `LIKE` clause, not just
/// the CLI path; `record_engagement_by_token` takes its token
/// from an unauthenticated web API route (`hq-web`'s notifications endpoint),
/// which is the more exposed caller.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Minimum id-prefix length accepted by `dismiss_by_id`. IDs are UUIDs; this
/// is generous enough for a real 8-char delivery token while ruling out a
/// one- or two-character prefix that could match many rows at once.
const MIN_ID_PREFIX_LEN: usize = 4;

/// Dismiss one active item (pending/routed/delivered) by id or id-prefix
/// (token). Returns true if a row matched. Rejects a prefix shorter than
/// [`MIN_ID_PREFIX_LEN`] rather than silently dismissing everything it
/// happens to match.
pub fn dismiss_by_id(db: &Database, id_or_token: &str) -> Result<bool> {
    if id_or_token.len() < MIN_ID_PREFIX_LEN {
        anyhow::bail!(
            "id/token '{id_or_token}' is too short (minimum {MIN_ID_PREFIX_LEN} characters) — \
             use `--all` to clear more than one item at a time"
        );
    }
    db.with_conn(|c| {
        let pattern = format!("{}%", escape_like(id_or_token));
        let n = c.execute(
            "UPDATE value_items SET state='dismissed', engaged_at=?2 \
             WHERE id LIKE ?1 ESCAPE '\\' AND state IN ('pending','routed','delivered')",
            params![pattern, ts(&Utc::now())],
        )?;
        Ok(n > 0)
    })
}

/// Bulk-dismiss active items, optionally scoped by kind and/or state. Always
/// restricted to pending/routed/delivered regardless of the `state` filter —
/// dismissing is a one-way transition out of the active set, never a way to
/// re-terminalize an already-engaged/dismissed/expired row (that would
/// destroy the engagement ledger). Returns rows affected.
pub fn dismiss_all(db: &Database, kind: Option<ValueKind>, state: Option<ValueState>) -> Result<usize> {
    db.with_conn(|c| {
        let now = ts(&Utc::now());
        let n = match (state, kind) {
            (Some(s), Some(k)) => c.execute(
                "UPDATE value_items SET state='dismissed', engaged_at=?1 \
                 WHERE state = ?2 AND state IN ('pending','routed','delivered') AND kind = ?3",
                params![now, s.as_str(), k.as_str()],
            )?,
            (Some(s), None) => c.execute(
                "UPDATE value_items SET state='dismissed', engaged_at=?1 \
                 WHERE state = ?2 AND state IN ('pending','routed','delivered')",
                params![now, s.as_str()],
            )?,
            (None, Some(k)) => c.execute(
                "UPDATE value_items SET state='dismissed', engaged_at=?1 \
                 WHERE state IN ('pending','routed','delivered') AND kind = ?2",
                params![now, k.as_str()],
            )?,
            (None, None) => c.execute(
                "UPDATE value_items SET state='dismissed', engaged_at=?1 \
                 WHERE state IN ('pending','routed','delivered')",
                params![now],
            )?,
        };
        Ok(n)
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskEngagement {
    pub source_task: String,
    pub delivered: i64,
    pub engaged: i64,
}

/// Per-task engagement ledger: how many items reached the user vs were acted on.
pub fn task_engagement_stats(db: &Database) -> Result<Vec<TaskEngagement>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT source_task, \
                SUM(CASE WHEN state IN ('delivered','engaged','dismissed') THEN 1 ELSE 0 END) AS delivered, \
                SUM(CASE WHEN state = 'engaged' THEN 1 ELSE 0 END) AS engaged \
             FROM value_items GROUP BY source_task ORDER BY delivered DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TaskEngagement {
                    source_task: r.get(0)?,
                    delivered: r.get(1)?,
                    engaged: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::ValueKind;

    fn open_db() -> Database {
        Database::open_memory().unwrap()
    }

    #[test]
    fn only_key_signed_approvals_count_as_signed() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("sg");
        let id = item.id.clone();
        emit(&db, &item).unwrap();
        assert!(!is_signed_approval(&db, "sg").unwrap());
        db.with_conn(|c| {
            c.execute("UPDATE value_items SET state='engaged', engagement='approved' WHERE id=?1", params![id])?;
            Ok(())
        })
        .unwrap();
        assert!(!is_signed_approval(&db, "sg").unwrap(), "a hand-edited row has no signature");
        db.with_conn(|c| {
            c.execute("UPDATE value_items SET state='delivered', engagement=NULL WHERE id=?1", params![id])?;
            Ok(())
        })
        .unwrap();
        assert!(approve_by_id(&db, &id).unwrap());
        assert!(is_signed_approval(&db, "sg").unwrap());
        db.with_conn(|c| {
            c.execute("UPDATE value_items SET engagement_mac='00' WHERE id=?1", params![id])?;
            Ok(())
        })
        .unwrap();
        assert!(!is_signed_approval(&db, "sg").unwrap(), "a wrong signature is refused");
    }

    #[test]
    fn approve_by_id_engages_active_items_once_and_rejects_short_prefixes() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("ap");
        let id = item.id.clone();
        emit(&db, &item).unwrap();
        assert_eq!(latest_state_by_dedup(&db, "ap").unwrap(), Some(ValueState::Pending));
        assert!(approve_by_id(&db, "ab").is_err(), "short prefix must not match everything");
        assert!(approve_by_id(&db, &id).unwrap());
        assert_eq!(latest_state_by_dedup(&db, "ap").unwrap(), Some(ValueState::Engaged));
        assert!(!approve_by_id(&db, &id).unwrap(), "an engaged item cannot be approved again");
        assert_eq!(latest_state_by_dedup(&db, "never-emitted").unwrap(), None);
    }

    #[test]
    fn dismissing_an_item_stops_the_same_dedup_key_from_regenerating() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Fyi, "a", "b").with_dedup_key("k");
        let id = item.id.clone();
        emit(&db, &item).unwrap();
        assert!(dismiss_by_id(&db, &id).unwrap());

        // The producer re-runs and tries to emit the same content again —
        // must collapse against the dismissed row, not insert a fresh one.
        let again = ValueItem::new("t", ValueKind::Fyi, "a", "b").with_dedup_key("k");
        emit(&db, &again).unwrap();

        assert_eq!(list_by_state(&db, ValueState::Pending).unwrap().len(), 0);
        assert_eq!(count_by_state(&db).unwrap(), vec![("dismissed".to_string(), 1)]);
    }

    #[test]
    fn dismiss_by_id_rejects_a_wildcard_prefix() {
        let db = open_db();
        emit(&db, &ValueItem::new("t", ValueKind::Fyi, "a", "b")).unwrap();
        emit(&db, &ValueItem::new("t", ValueKind::Fyi, "c", "d")).unwrap();

        // Would otherwise LIKE-match every id and dismiss the whole queue.
        assert!(dismiss_by_id(&db, "%").is_err());
        assert_eq!(list_by_state(&db, ValueState::Pending).unwrap().len(), 2);
    }

    #[test]
    fn dismiss_all_with_a_terminal_state_filter_touches_nothing() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Fyi, "a", "b");
        let id = item.id.clone();
        insert(&db, &item).unwrap();
        set_delivered(&db, &id).unwrap();
        let token: String = id.chars().take(8).collect();
        record_engagement_by_token(&db, &token, "approved").unwrap();

        // Asking to "clear" already-engaged items must not flip them to
        // dismissed and wipe the engagement ledger.
        let n = dismiss_all(&db, None, Some(ValueState::Engaged)).unwrap();
        assert_eq!(n, 0);
        assert_eq!(list_by_state(&db, ValueState::Engaged).unwrap().len(), 1);
    }

    #[test]
    fn insert_and_list_pending() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::ActionNeeded, "title", "body");
        insert(&db, &item).unwrap();
        let pending = list_by_state(&db, ValueState::Pending).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].title, "title");
    }

    #[test]
    fn emit_collapses_duplicate_dedup_key() {
        let db = open_db();
        let a = ValueItem::new("t", ValueKind::Insight, "a", "b").with_dedup_key("k");
        let b = ValueItem::new("t", ValueKind::Insight, "c", "d").with_dedup_key("k");
        emit(&db, &a).unwrap();
        emit(&db, &b).unwrap();
        assert_eq!(list_by_state(&db, ValueState::Pending).unwrap().len(), 1);
    }

    #[test]
    fn list_pending_sorted_by_score_desc() {
        let db = open_db();
        emit(&db, &ValueItem::new("t", ValueKind::Fyi, "low", "b")).unwrap();
        emit(
            &db,
            &ValueItem::new("t", ValueKind::ActionNeeded, "high", "b"),
        )
        .unwrap();
        let pending = list_by_state(&db, ValueState::Pending).unwrap();
        assert_eq!(pending[0].title, "high");
    }

    #[test]
    fn deliver_then_engage_by_token() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Proposal, "p", "b");
        let id = item.id.clone();
        insert(&db, &item).unwrap();
        set_delivered(&db, &id).unwrap();
        let token: String = id.chars().take(8).collect();
        assert!(record_engagement_by_token(&db, &token, "approved").unwrap());
        let stats = task_engagement_stats(&db).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].delivered, 1);
        assert_eq!(stats[0].engaged, 1);
    }

    #[test]
    fn engage_only_matches_delivered_items() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Fyi, "p", "b");
        let token: String = item.id.chars().take(8).collect();
        insert(&db, &item).unwrap(); // still pending, not delivered
        assert!(!record_engagement_by_token(&db, &token, "approved").unwrap());
    }

    #[test]
    fn record_engagement_by_token_rejects_a_wildcard_token() {
        // Regression: an unauthenticated web API route passes a URL path
        // segment straight into this function. `%` would otherwise match
        // and mass-engage every delivered row in one request.
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Fyi, "p", "b");
        let id = item.id.clone();
        insert(&db, &item).unwrap();
        set_delivered(&db, &id).unwrap();

        assert!(!record_engagement_by_token(&db, "%", "approved").unwrap());
        assert!(!record_engagement_by_token(&db, "_", "approved").unwrap());

        // The real token still works after the wildcard attempts.
        let token: String = id.chars().take(8).collect();
        assert!(record_engagement_by_token(&db, &token, "approved").unwrap());
    }

    #[test]
    fn gc_expires_only_old_pending() {
        let db = open_db();
        let mut old = ValueItem::new("t", ValueKind::Fyi, "old", "b");
        old.expires_at = Some(Utc::now() - chrono::Duration::days(1));
        insert(&db, &old).unwrap();
        let fresh = ValueItem::new("t", ValueKind::Fyi, "fresh", "b");
        insert(&db, &fresh).unwrap();
        assert_eq!(gc_expired(&db).unwrap(), 1);
        assert_eq!(list_by_state(&db, ValueState::Pending).unwrap().len(), 1);
        assert_eq!(list_by_state(&db, ValueState::Expired).unwrap().len(), 1);
    }
}
