use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::types::{ValueItem, ValueKind, ValueState};
use rusqlite::{OptionalExtension, Row, params};

use crate::Database;

const COLS: &str = "id, source_task, kind, title, body, artifact_path, score, dedup_key, \
                    state, created_at, routed_at, delivered_at, engaged_at, expires_at, engagement, link, owner_only";

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
        link: r.get(15)?,
        owner_only: r.get::<_, i64>(16)? != 0,
    })
}

/// Raw insert. Prefer `emit` which applies dedup.
pub fn insert(db: &Database, item: &ValueItem) -> Result<()> {
    db.with_conn(|c| insert_on(c, item))
}

/// [`insert`] on a connection the caller holds, so it can share a transaction.
pub fn insert_on(c: &rusqlite::Connection, item: &ValueItem) -> Result<()> {
    c.execute(
        "INSERT INTO value_items (id, source_task, kind, title, body, artifact_path, score, \
         dedup_key, state, created_at, routed_at, delivered_at, engaged_at, expires_at, engagement, link, owner_only) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
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
            item.link,
            item.owner_only,
        ],
    )?;
    Ok(())
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

/// Record engagement on the one delivered item whose id starts with `token`, the short ref a
/// chat button or a typed `approve <token>` carries. Returns true if a row matched. `outcome` is
/// "approved" or "dismissed". A token that matches more than one delivered item is refused
/// rather than answering all of them: an approval approves exactly one request.
pub fn record_engagement_by_token(db: &Database, token: &str, outcome: &str) -> Result<bool> {
    // A short or wildcard-only token must never become "match every delivered row".
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
        let [id] = ids.as_slice() else {
            if ids.len() > 1 {
                anyhow::bail!(
                    "'{token}' matches {} items; use the full id from the inbox",
                    ids.len()
                );
            }
            return Ok(false);
        };
        let n = c.execute(
            "UPDATE value_items SET state=?3, engagement=?2, engaged_at=?4, engagement_mac=NULL \
             WHERE id = ?1 AND state='delivered'",
            params![id, outcome, state, ts(&Utc::now())],
        )?;
        if n > 0 && state == "engaged" {
            sign_approvals(c, std::slice::from_ref(id));
        }
        Ok(n > 0)
    })
}

/// How answering one item by its exact id went ([`answer_by_id`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// The item was waiting and now carries the answer; an approval is signed.
    Recorded,
    /// The item already carried this same answer. Nothing was written.
    AlreadySo,
    /// The item was answered the other way, expired, or its approval was spent. Nothing was
    /// written, so a spent approval is never re-armed.
    Settled(ValueState),
    NotFound,
}

/// Answer the one item whose id is exactly `id`, while it waits (pending, routed or delivered).
/// `outcome` is "approved" (signed with the install key, like every other owner approval) or
/// "dismissed". The web inbox and the phone send the full id, so no prefix is ever matched here.
pub fn answer_by_id(db: &Database, id: &str, outcome: &str) -> Result<Answered> {
    let state = match outcome {
        "approved" => ValueState::Engaged,
        "dismissed" => ValueState::Dismissed,
        other => anyhow::bail!("unknown outcome '{other}'"),
    };
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET state=?2, engagement=?3, engaged_at=?4, engagement_mac=NULL \
             WHERE id = ?1 AND state IN ('pending','routed','delivered')",
            params![id, state.as_str(), outcome, ts(&Utc::now())],
        )?;
        if n > 0 {
            if state == ValueState::Engaged {
                sign_approvals(c, &[id.to_string()]);
            }
            return Ok(Answered::Recorded);
        }
        let current: Option<(String, Option<String>)> = c
            .query_row(
                "SELECT state, engagement FROM value_items WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((current, engagement)) = current else {
            return Ok(Answered::NotFound);
        };
        let current = ValueState::from_str(&current).unwrap_or(ValueState::Expired);
        // A spent approval reads `used`: approving it again is not "already so", it is over.
        Ok(
            if current == state && engagement.as_deref() == Some(outcome) {
                Answered::AlreadySo
            } else {
                Answered::Settled(current)
            },
        )
    })
}

/// Ids matching `pattern` whose state is in `states` (a pre-quoted list body).
fn matching_ids(c: &rusqlite::Connection, pattern: &str, states: &str) -> Result<Vec<String>> {
    // `states` is only ever one of this file's own literals.
    let sql = format!(
        "SELECT id FROM value_items WHERE id LIKE ?1 ESCAPE '\\' AND state IN ('{states}')"
    );
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
        let Ok(dedup_key) = c.query_row(
            "SELECT dedup_key FROM value_items WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        ) else {
            continue;
        };
        let mac = hq_core::approval_key::hmac_hex(
            &key,
            &hq_core::approval_key::value_approval_message(id, dedup_key.as_deref()),
        );
        let _ = c.execute(
            "UPDATE value_items SET engagement_mac = ?2 WHERE id = ?1",
            params![id, mac],
        );
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
                &hq_core::approval_key::value_approval_message(&id, Some(dedup_key)),
                &mac,
            ),
            _ => false,
        })
    })
}

/// Engagement that marks a signed approval as spent by the one action it approved.
pub const ENGAGEMENT_USED: &str = "used";

/// The newest item with a dedup key, as an approval gate reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestEngagement {
    pub state: ValueState,
    pub engagement: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// State, engagement and expiry of the newest item with this dedup key, in any state.
pub fn latest_engagement_by_dedup(
    db: &Database,
    dedup_key: &str,
) -> Result<Option<LatestEngagement>> {
    db.with_conn(|c| {
        let row: Option<(String, Option<String>, Option<String>)> = c
            .query_row(
                "SELECT state, engagement, expires_at FROM value_items WHERE dedup_key = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![dedup_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(s, engagement, exp)| {
            ValueState::from_str(&s).map(|state| LatestEngagement {
                state,
                engagement,
                expires_at: parse_opt_ts(exp),
            })
        }))
    })
}

/// Where the newest request with one dedup key stands, for a producer deciding whether to ask
/// the owner (again).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// No live request: send one.
    Ask,
    Wait,
    Declined,
}

impl RequestState {
    /// An expired request, answered or not, is asked again: the owner may have missed it, and a
    /// decline lasts as long as the request does. A spent approval is asked again too.
    pub fn from_latest(latest: Option<LatestEngagement>, now: DateTime<Utc>) -> Self {
        let Some(latest) = latest else {
            return Self::Ask;
        };
        if latest.expires_at.is_some_and(|at| at <= now) {
            return Self::Ask;
        }
        let spent = latest.engagement.as_deref() == Some(ENGAGEMENT_USED);
        match latest.state {
            ValueState::Engaged if spent => Self::Ask,
            ValueState::Expired => Self::Ask,
            ValueState::Dismissed => Self::Declined,
            _ => Self::Wait,
        }
    }
}

/// Where the owner's approval for one dedup key stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalGate {
    /// A signed approval was there and this call spent it.
    Run,
    /// Nothing to spend: ask, keep waiting, or report the decline.
    Pending(RequestState),
}

/// Reads where the approval for `dedup_key` stands, spending a signed approval when one is
/// there, so each approval lets exactly one call through.
pub fn approval_gate(db: &Database, dedup_key: &str) -> Result<ApprovalGate> {
    let latest = latest_engagement_by_dedup(db, dedup_key)?;
    if consume_approval(db, dedup_key)? {
        return Ok(ApprovalGate::Run);
    }
    Ok(ApprovalGate::Pending(RequestState::from_latest(
        latest,
        Utc::now(),
    )))
}

/// Items still waiting on the owner whose dedup key starts with `prefix` and that have not
/// expired, for a producer that caps how many it has open at once.
pub fn count_waiting_with_prefix(db: &Database, prefix: &str) -> Result<i64> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM value_items
             WHERE dedup_key LIKE ?1 ESCAPE '\\' AND state IN ('pending','routed','delivered')
               AND (expires_at IS NULL OR expires_at > ?2)",
            params![format!("{}%", escape_like(prefix)), ts(&Utc::now())],
            |r| r.get(0),
        )?)
    })
}

/// Spends the newest signed approval for `dedup_key`: true for the one caller that may act on
/// it, false when there is none or it was already spent.
pub fn consume_approval(db: &Database, dedup_key: &str) -> Result<bool> {
    if !is_signed_approval(db, dedup_key)? {
        return Ok(false);
    }
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET engagement = ?2
             WHERE id = (SELECT id FROM value_items WHERE dedup_key = ?1
                         ORDER BY created_at DESC, rowid DESC LIMIT 1)
               AND state = 'engaged' AND engagement = 'approved'
               AND (expires_at IS NULL OR expires_at > ?3)",
            params![dedup_key, ENGAGEMENT_USED, ts(&Utc::now())],
        )?;
        Ok(n == 1)
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
/// operators without a chat relay (`hq queue approve`). Same length rule as
/// [`dismiss_by_id`]; a prefix that matches more than one active item is refused.
pub fn approve_by_id(db: &Database, id_or_token: &str) -> Result<bool> {
    if id_or_token.len() < MIN_ID_PREFIX_LEN {
        anyhow::bail!(
            "id/token '{id_or_token}' is too short (minimum {MIN_ID_PREFIX_LEN} characters)"
        );
    }
    db.with_conn(|c| {
        let pattern = format!("{}%", escape_like(id_or_token));
        let ids = matching_ids(c, &pattern, "pending','routed','delivered")?;
        let [id] = ids.as_slice() else {
            if ids.len() > 1 {
                anyhow::bail!(
                    "'{id_or_token}' matches {} items; give more of the id",
                    ids.len()
                );
            }
            return Ok(false);
        };
        let n = c.execute(
            "UPDATE value_items SET state='engaged', engagement='approved', engaged_at=?2, \
             engagement_mac=NULL WHERE id = ?1 AND state IN ('pending','routed','delivered')",
            params![id, ts(&Utc::now())],
        )?;
        if n > 0 {
            sign_approvals(c, std::slice::from_ref(id));
        }
        Ok(n > 0)
    })
}

/// Retires one active item whose condition went away by itself (a host that answers again), so
/// it leaves the inbox without counting as the owner's dismissal. True if a row matched.
pub fn retire(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET state='expired' \
             WHERE id = ?1 AND state IN ('pending','routed','delivered')",
            params![id],
        )?;
        Ok(n > 0)
    })
}

/// Whether any item, in any state, ever carried `dedup_key`. For a producer that must say a
/// thing at most once however the owner answered it.
pub fn any_with_dedup(db: &Database, dedup_key: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM value_items WHERE dedup_key = ?1)",
            params![dedup_key],
            |r| r.get(0),
        )?)
    })
}

/// Like [`any_with_dedup`], but only counts items written by one of `sources`, so another
/// producer's item with the same key cannot stand in for the supervisor's own.
pub fn any_with_dedup_from(db: &Database, dedup_key: &str, sources: &[&str]) -> Result<bool> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT source_task FROM value_items WHERE dedup_key = ?1")?;
        let found = stmt
            .query_map(params![dedup_key], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(found.iter().any(|s| sources.contains(&s.as_str())))
    })
}

/// Retires every waiting item whose dedup key is `prefix` followed by a number: the alerts of
/// one session's blocks, once it is no longer blocked. Returns how many left the inbox.
pub fn retire_waiting_numbered(db: &Database, prefix: &str) -> Result<usize> {
    db.with_conn(|c| {
        let waiting: Vec<(String, String)> = c
            .prepare(
                "SELECT id, dedup_key FROM value_items
                 WHERE dedup_key LIKE ?1 ESCAPE '\\' AND state IN ('pending','routed','delivered')",
            )?
            .query_map(params![format!("{}%", escape_like(prefix))], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut retired = 0;
        for (id, key) in waiting {
            let rest = &key[prefix.len()..];
            if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            retired += c.execute(
                "UPDATE value_items SET state='expired' \
                 WHERE id = ?1 AND state IN ('pending','routed','delivered')",
                params![id],
            )?;
        }
        Ok(retired)
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
        let sql = format!(
            "SELECT {COLS} FROM value_items {where_clause} ORDER BY created_at DESC LIMIT ?{}",
            clauses.len() + 1
        );
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
/// the CLI path; `record_engagement_by_token` takes its token from a chat
/// button or message. The web inbox answers by exact id ([`answer_by_id`]).
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Minimum id-prefix length accepted by `dismiss_by_id`. IDs are UUIDs; this
/// is generous enough for a real 8-char delivery token while ruling out a
/// one- or two-character prefix that could match many rows at once.
const MIN_ID_PREFIX_LEN: usize = 4;

/// Dismiss one active item (pending/routed/delivered) by id or id-prefix
/// (token). Returns true if a row matched. Rejects a prefix shorter than
/// [`MIN_ID_PREFIX_LEN`] rather than silently dismissing everything it
/// happens to match.
/// Undo a clear: puts a dismissed item back while the clear is at most `window_secs` old
/// (resolutions.md A6). Returns false when it is older, was answered instead, or is unknown.
pub fn restore_dismissed(db: &Database, id: &str, window_secs: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET state='delivered', engagement=NULL, engaged_at=NULL, cleared_at=NULL \
             WHERE id = ?1 AND state = 'dismissed' AND kind NOT IN ('action_needed', 'proposal') \
               AND julianday(engaged_at) >= julianday('now', ?2)",
            params![id, format!("-{window_secs} seconds")],
        )?;
        Ok(n > 0)
    })
}

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
pub fn dismiss_all(
    db: &Database,
    kind: Option<ValueKind>,
    state: Option<ValueState>,
) -> Result<usize> {
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

/// Items created since `since` that no push dispatcher has taken yet, oldest first. Older
/// ones are left alone, so turning push on never replays a backlog.
pub fn list_unpushed(db: &Database, since: &DateTime<Utc>) -> Result<Vec<ValueItem>> {
    db.with_conn(|c| {
        let sql = format!(
            "SELECT {COLS} FROM value_items WHERE pushed_at IS NULL AND created_at >= ?1 \
             ORDER BY created_at ASC LIMIT 50"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(params![ts(since)], row_to_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Takes `id` for pushing. Exactly one caller gets `true`, however many processes ask.
pub fn claim_push(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE value_items SET pushed_at = ?2 WHERE id = ?1 AND pushed_at IS NULL",
            params![id, ts(&Utc::now())],
        )?;
        Ok(n == 1)
    })
}

/// How many items any of `sources` created since `since`.
pub fn count_from_sources_since(
    db: &Database,
    sources: &[&str],
    since: &DateTime<Utc>,
) -> Result<usize> {
    db.with_conn(|c| {
        let mut total = 0usize;
        for source in sources {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM value_items WHERE source_task = ?1 AND created_at >= ?2",
                params![source, ts(since)],
                |r| r.get(0),
            )?;
            total += n as usize;
        }
        Ok(total)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_is_undone_within_the_window_and_not_after() {
        let db = Database::open_memory().unwrap();
        let item = ValueItem::new("agent_notify", ValueKind::Fyi, "t", "b");
        emit(&db, &item).unwrap();
        assert!(dismiss_by_id(&db, &item.id).unwrap());
        assert!(restore_dismissed(&db, &item.id, 600).unwrap());
        assert!(
            !restore_dismissed(&db, &item.id, 600).unwrap(),
            "only a cleared item"
        );
        assert!(dismiss_by_id(&db, &item.id).unwrap());
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET engaged_at = datetime('now', '-11 minutes') WHERE id = ?1",
                params![item.id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(!restore_dismissed(&db, &item.id, 600).unwrap(), "too late");
    }

    #[test]
    fn a_declined_approval_never_comes_back_through_undo() {
        let db = Database::open_memory().unwrap();
        let item = ValueItem::new("owner-approval", ValueKind::ActionNeeded, "t", "b");
        emit(&db, &item).unwrap();
        assert!(dismiss_by_id(&db, &item.id).unwrap());
        assert!(!restore_dismissed(&db, &item.id, 600).unwrap());
    }
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
            c.execute(
                "UPDATE value_items SET state='engaged', engagement='approved' WHERE id=?1",
                params![id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            !is_signed_approval(&db, "sg").unwrap(),
            "a hand-edited row has no signature"
        );
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET state='delivered', engagement=NULL WHERE id=?1",
                params![id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(approve_by_id(&db, &id).unwrap());
        assert!(is_signed_approval(&db, "sg").unwrap());
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET engagement_mac='00' WHERE id=?1",
                params![id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            !is_signed_approval(&db, "sg").unwrap(),
            "a wrong signature is refused"
        );
    }

    #[test]
    fn a_signed_approval_cannot_be_moved_to_another_dedup_key() {
        let db = open_db();
        let item =
            ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("call:aaa");
        emit(&db, &item).unwrap();
        assert!(approve_by_id(&db, &item.id).unwrap());
        assert!(is_signed_approval(&db, "call:aaa").unwrap());
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET dedup_key='call:bbb' WHERE id=?1",
                params![item.id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            !is_signed_approval(&db, "call:bbb").unwrap(),
            "the signature covers the key it was given for"
        );
        assert!(!consume_approval(&db, "call:bbb").unwrap());
    }

    #[test]
    fn a_signed_approval_is_spent_once_and_a_new_request_can_follow() {
        let db = open_db();
        let first = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("once");
        emit(&db, &first).unwrap();
        assert!(
            !consume_approval(&db, "once").unwrap(),
            "nothing approved yet"
        );
        assert!(approve_by_id(&db, &first.id).unwrap());
        assert!(consume_approval(&db, "once").unwrap());
        assert!(!consume_approval(&db, "once").unwrap(), "spent");
        let mut late =
            ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("late");
        late.expires_at = Some(Utc::now() - chrono::Duration::minutes(1));
        insert(&db, &late).unwrap();
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET state='delivered' WHERE id=?1",
                params![late.id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(approve_by_id(&db, &late.id).unwrap());
        assert!(
            !consume_approval(&db, "late").unwrap(),
            "an approval ends with its request"
        );
        let latest = latest_engagement_by_dedup(&db, "once").unwrap().unwrap();
        assert_eq!(latest.state, ValueState::Engaged);
        assert_eq!(latest.engagement.as_deref(), Some(ENGAGEMENT_USED));
        let second = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("once");
        emit(&db, &second).unwrap();
        assert_eq!(
            latest_state_by_dedup(&db, "once").unwrap(),
            Some(ValueState::Pending),
            "a spent approval does not swallow the next request"
        );
    }

    #[test]
    fn waiting_items_are_counted_by_prefix_until_answered_or_expired() {
        let db = open_db();
        for key in ["connector:g1:a", "connector:g1:b", "connector:g2:a"] {
            let item = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key(key);
            emit(&db, &item).unwrap();
        }
        let mut stale =
            ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("connector:g1:c");
        stale.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
        emit(&db, &stale).unwrap();
        assert_eq!(count_waiting_with_prefix(&db, "connector:g1:").unwrap(), 2);
        assert_eq!(
            count_waiting_with_prefix(&db, "connector:g_:").unwrap(),
            0,
            "no LIKE wildcards"
        );
        let first = find_active_by_dedup(&db, ValueKind::ActionNeeded, "connector:g1:a")
            .unwrap()
            .unwrap();
        assert!(dismiss_by_id(&db, &first.id).unwrap());
        assert_eq!(count_waiting_with_prefix(&db, "connector:g1:").unwrap(), 1);
    }

    #[test]
    fn approve_by_id_engages_active_items_once_and_rejects_short_prefixes() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("ap");
        let id = item.id.clone();
        emit(&db, &item).unwrap();
        assert_eq!(
            latest_state_by_dedup(&db, "ap").unwrap(),
            Some(ValueState::Pending)
        );
        assert!(
            approve_by_id(&db, "ab").is_err(),
            "short prefix must not match everything"
        );
        assert!(approve_by_id(&db, &id).unwrap());
        assert_eq!(
            latest_state_by_dedup(&db, "ap").unwrap(),
            Some(ValueState::Engaged)
        );
        assert!(
            !approve_by_id(&db, &id).unwrap(),
            "an engaged item cannot be approved again"
        );
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
        assert_eq!(
            count_by_state(&db).unwrap(),
            vec![("dismissed".to_string(), 1)]
        );
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
        let engaged = list_by_state(&db, ValueState::Engaged).unwrap();
        assert_eq!(engaged.len(), 1);
        assert_eq!(engaged[0].id, id);
    }

    #[test]
    fn engage_only_matches_delivered_items() {
        let db = open_db();
        let item = ValueItem::new("t", ValueKind::Fyi, "p", "b");
        let token: String = item.id.chars().take(8).collect();
        insert(&db, &item).unwrap(); // still pending, not delivered
        assert!(!record_engagement_by_token(&db, &token, "approved").unwrap());
    }

    /// A short ref that names two waiting items answers neither: an approval approves exactly
    /// one request, so the person has to say which.
    #[test]
    fn an_ambiguous_prefix_approves_nothing() {
        let db = open_db();
        for (id, dedup) in [("cafe0001-a", "ask:a"), ("cafe0002-b", "ask:b")] {
            let mut item =
                ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key(dedup);
            item.id = id.into();
            insert(&db, &item).unwrap();
            set_delivered(&db, id).unwrap();
        }
        assert!(record_engagement_by_token(&db, "cafe", "approved").is_err());
        assert!(approve_by_id(&db, "cafe").is_err());
        assert_eq!(list_by_state(&db, ValueState::Engaged).unwrap().len(), 0);
        assert!(!is_signed_approval(&db, "ask:a").unwrap());
        assert!(!is_signed_approval(&db, "ask:b").unwrap());

        assert!(record_engagement_by_token(&db, "cafe0001", "approved").unwrap());
        assert!(is_signed_approval(&db, "ask:a").unwrap());
        assert!(!is_signed_approval(&db, "ask:b").unwrap());
        assert!(
            approve_by_id(&db, "cafe").unwrap(),
            "the one item still waiting is no longer ambiguous"
        );
        assert!(is_signed_approval(&db, "ask:b").unwrap());
    }

    #[test]
    fn answer_by_id_matches_the_whole_id_and_never_reopens_a_settled_item() {
        let db = open_db();
        let mut item =
            ValueItem::new("t", ValueKind::ActionNeeded, "a", "b").with_dedup_key("ask:x");
        item.id = "beef0001-x".into();
        insert(&db, &item).unwrap();

        assert_eq!(
            answer_by_id(&db, "beef", "approved").unwrap(),
            Answered::NotFound
        );
        assert_eq!(
            answer_by_id(&db, "beef0001-x", "approved").unwrap(),
            Answered::Recorded,
            "a pending item is answered, not only a delivered one"
        );
        assert!(is_signed_approval(&db, "ask:x").unwrap());
        assert_eq!(
            answer_by_id(&db, "beef0001-x", "approved").unwrap(),
            Answered::AlreadySo
        );
        assert!(consume_approval(&db, "ask:x").unwrap());
        assert_eq!(
            answer_by_id(&db, "beef0001-x", "approved").unwrap(),
            Answered::Settled(ValueState::Engaged)
        );
        assert!(
            !consume_approval(&db, "ask:x").unwrap(),
            "a spent approval is not re-armed"
        );
        assert_eq!(
            answer_by_id(&db, "beef0001-x", "dismissed").unwrap(),
            Answered::Settled(ValueState::Engaged)
        );
        assert!(answer_by_id(&db, "beef0001-x", "bogus").is_err());
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

    #[test]
    fn link_round_trips_and_new_items_wait_for_one_push_claim() {
        let db = open_db();
        let since = Utc::now() - chrono::Duration::minutes(1);
        let item =
            ValueItem::new("agent_notify", ValueKind::Fyi, "Deploy done", "ok").with_link("/tasks");
        emit(&db, &item).unwrap();

        let unpushed = list_unpushed(&db, &since).unwrap();
        assert_eq!(unpushed.len(), 1);
        assert_eq!(unpushed[0].link.as_deref(), Some("/tasks"));
        assert!(claim_push(&db, &item.id).unwrap());
        assert!(!claim_push(&db, &item.id).unwrap(), "a second claim loses");
        assert!(list_unpushed(&db, &since).unwrap().is_empty());
    }

    #[test]
    fn unpushed_items_older_than_the_window_are_not_listed() {
        let db = open_db();
        let mut old = ValueItem::new("agent_notify", ValueKind::Fyi, "old", "b");
        old.created_at = Utc::now() - chrono::Duration::hours(3);
        insert(&db, &old).unwrap();
        let since = Utc::now() - chrono::Duration::hours(1);
        assert!(list_unpushed(&db, &since).unwrap().is_empty());
    }

    #[test]
    fn counts_items_from_the_named_sources_in_the_window() {
        let db = open_db();
        let since = Utc::now() - chrono::Duration::hours(1);
        for source in ["agent_notify", "agent_notify_relay", "email-triage"] {
            emit(&db, &ValueItem::new(source, ValueKind::Fyi, "t", "b")).unwrap();
        }
        let n =
            count_from_sources_since(&db, &["agent_notify", "agent_notify_relay"], &since).unwrap();
        assert_eq!(n, 2);
    }
}
