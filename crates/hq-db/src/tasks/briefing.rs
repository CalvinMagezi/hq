//! Queue health and the exception-first briefing (docs/plans/task-verification.md, "Weekend
//! bursts and the Monday briefing").
//!
//! The briefing leads with what needs the owner, then what was verified and completed (grouped,
//! never one pass at a time), what failed and was reworked without them, tier 2 tasks inside
//! their veto window, and a suggested audit set. Past `owner_queue_alert` items it stops listing
//! and offers a bulk policy, because a long flat list is the bottleneck this exists to remove.

use super::verification::{LEVEL_OWNER, VERDICT_PASS};
use super::waits::{WAIT_BUDGET, WAIT_CI, WAIT_OWNER, WAIT_VERIFIER, WAIT_WORKER, WaitingItem};
use super::*;
use hq_core::config::{VerificationConfig, VerificationMode};
use std::collections::BTreeMap;

/// Tasks named per section; the count still covers all of them.
const LISTED: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Deferral {
    pub waiting_for: String,
    pub reason: String,
    pub tasks: i64,
}

/// How the verification queue is doing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueueHealth {
    pub mode: VerificationMode,
    /// Tasks per party they wait on: worker, ci, verifier, budget, owner.
    pub backlog: BTreeMap<String, i64>,
    /// What the owner's share is made of, by kind.
    pub owner_by_kind: BTreeMap<String, i64>,
    pub oldest_owner_hours: Option<i64>,
    /// Verdicts produced in the last 24 hours.
    pub verdicts_24h: i64,
    /// Tasks HQ completed in the last 24 hours.
    pub completed_24h: i64,
    /// Hours to clear what waits on the verifier at the rate of the last 24 hours.
    pub est_clear_hours: Option<f64>,
    pub deferrals: Vec<Deferral>,
    pub spent_this_month_usd: f64,
    pub monthly_ceiling_usd: Option<f64>,
    /// Plain sentences, at most one per cause. Empty when nothing needs attention.
    pub alerts: Vec<String>,
}

fn hours_since(stamp: &str) -> Option<i64> {
    let then = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").ok()?;
    Some((chrono::Utc::now().naive_utc() - then).num_hours().max(0))
}

pub fn queue_health(conn: &Connection, cfg: &VerificationConfig) -> Result<QueueHealth> {
    let items = waiting_items(conn)?;
    let mut backlog: BTreeMap<String, i64> = BTreeMap::new();
    let mut owner_by_kind: BTreeMap<String, i64> = BTreeMap::new();
    let mut oldest_owner: Option<i64> = None;
    for item in &items {
        *backlog.entry(item.who.clone()).or_insert(0) += 1;
        if item.who == WAIT_OWNER {
            *owner_by_kind.entry(item.kind.to_string()).or_insert(0) += 1;
            if let Some(h) = hours_since(&item.since) {
                oldest_owner = Some(oldest_owner.map_or(h, |o| o.max(h)));
            }
        }
    }
    let (verdicts_24h, completed_24h): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(verdict = ?1 AND applied = 1 AND level <> ?2), 0) \
         FROM task_verdicts WHERE canary = 0 AND created_at >= datetime('now', '-1 day')",
        params![VERDICT_PASS, LEVEL_OWNER],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let verifier_backlog = backlog.get(WAIT_VERIFIER).copied().unwrap_or(0);
    let est_clear_hours = (verdicts_24h > 0 && verifier_backlog > 0)
        .then(|| (verifier_backlog as f64 / (verdicts_24h as f64 / 24.0) * 10.0).round() / 10.0);

    let mut stmt = conn.prepare(
        "SELECT w.waiting_for, substr(w.reason, 1, 90), COUNT(*) FROM task_verification_waits w \
         JOIN tasks t ON t.id = w.task_id WHERE t.status = 'ready_for_review' AND t.archived_at IS NULL \
         GROUP BY w.waiting_for, substr(w.reason, 1, 90) ORDER BY COUNT(*) DESC LIMIT 10",
    )?;
    let deferrals = stmt
        .query_map([], |r| {
            Ok(Deferral {
                waiting_for: r.get(0)?,
                reason: r.get(1)?,
                tasks: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut alerts = Vec::new();
    let owner_waiting = backlog.get(WAIT_OWNER).copied().unwrap_or(0);
    if cfg.mode != VerificationMode::Off {
        if owner_waiting > i64::from(cfg.owner_queue_alert) {
            alerts.push(format!(
                "{owner_waiting} items wait on you, over {}: decide them by group instead of one by one",
                cfg.owner_queue_alert
            ));
        }
        if let Some(h) = oldest_owner
            && h >= i64::from(cfg.owner_queue_age_days) * 24
        {
            alerts.push(format!(
                "the oldest item waiting on you is {} days old",
                h / 24
            ));
        }
        if let Some(h) = est_clear_hours
            && h > 24.0
        {
            alerts.push(format!(
                "at the current rate the verification queue takes {h:.0} hours to clear"
            ));
        }
        if let Some(ceiling) = cfg.reviewer_monthly_ceiling_usd {
            let spent = reviewer_spend_this_month(conn)?;
            if spent >= ceiling && backlog.get(WAIT_BUDGET).copied().unwrap_or(0) > 0 {
                alerts.push(format!(
                    "the reviewer budget (${ceiling:.2} a month) is used up, so some tasks wait: machine checks continue, raise the ceiling for the rest"
                ));
            }
        }
    }
    Ok(QueueHealth {
        mode: cfg.mode,
        backlog,
        owner_by_kind,
        oldest_owner_hours: oldest_owner,
        verdicts_24h,
        completed_24h,
        est_clear_hours,
        deferrals,
        spent_this_month_usd: reviewer_spend_this_month(conn)?,
        monthly_ceiling_usd: cfg.reviewer_monthly_ceiling_usd,
        alerts,
    })
}

/// Tasks HQ completed since the briefing's start, one line per initiative.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompletedGroup {
    pub initiative_id: String,
    pub initiative: String,
    pub count: i64,
    /// Evidence level to count: machine_peer, peer, machine.
    pub by_level: BTreeMap<String, i64>,
    pub tier2: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReworkedTask {
    pub task_id: String,
    pub display_id: String,
    pub title: String,
    pub returns: i64,
    pub last_finding: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VetoWindow {
    pub task_id: String,
    pub display_id: String,
    pub title: String,
    pub release_at: i64,
}

/// When many things wait on the owner: one decision for the group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BulkOffer {
    pub count: i64,
    pub text: String,
    /// The command that does it.
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Briefing {
    pub since: String,
    pub mode: VerificationMode,
    /// What needs the owner, oldest first, at most [`LISTED`] (or `owner_queue_alert`).
    pub needs_you: Vec<WaitingItem>,
    pub needs_you_total: i64,
    pub bulk_offer: Option<BulkOffer>,
    pub completed: Vec<CompletedGroup>,
    pub completed_total: i64,
    pub reworked: Vec<ReworkedTask>,
    pub reworked_total: i64,
    pub veto_windows: Vec<VetoWindow>,
    /// Completed tasks nobody has audited yet, and audits already waiting.
    pub audit_pool: i64,
    pub open_audits: i64,
    pub health: QueueHealth,
}

/// Builds the briefing for what happened since `since` (UTC `YYYY-MM-DD HH:MM:SS`).
pub fn briefing(conn: &Connection, cfg: &VerificationConfig, since: &str) -> Result<Briefing> {
    let health = queue_health(conn, cfg)?;
    let all = waiting_items(conn)?;
    let owner_items: Vec<WaitingItem> = all.into_iter().filter(|i| i.who == WAIT_OWNER).collect();
    let needs_you_total = owner_items.len() as i64;
    let cap = (cfg.owner_queue_alert as usize).clamp(1, LISTED);

    // Past the threshold, the list is replaced by an offer to decide the biggest group at once.
    let bulk_offer = (cfg.mode != VerificationMode::Off
        && needs_you_total > i64::from(cfg.owner_queue_alert))
    .then(|| {
        let mut by_kind: BTreeMap<&str, i64> = BTreeMap::new();
        for i in &owner_items {
            *by_kind.entry(i.kind).or_insert(0) += 1;
        }
        let (kind, n) = by_kind
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, n)| (*k, *n))
            .unwrap_or(("review", 0));
        match kind {
            "legacy" => BulkOffer {
                count: n,
                text: format!(
                    "{n} of {needs_you_total} were submitted before verification was enforced. A sweep reads their evidence once, shows you the groups, and applies the default for each on one confirmation."
                ),
                command: "hq verify sweep plan".to_string(),
            },
            "held" => BulkOffer {
                count: n,
                text: format!(
                    "{n} of {needs_you_total} passed but are held for you because the verifier's calibration is below the bar. Read hq verify status before releasing them."
                ),
                command: "hq verify status".to_string(),
            },
            _ => BulkOffer {
                count: n,
                text: format!(
                    "{n} of {needs_you_total} are waiting for your review. Turn on verification for the tier, or sweep them."
                ),
                command: "hq verify sweep plan".to_string(),
            },
        }
    });
    let needs_you = if bulk_offer.is_some() {
        Vec::new()
    } else {
        owner_items.into_iter().take(cap).collect()
    };

    // Completed by HQ since then, grouped.
    let mut stmt = conn.prepare(
        "SELECT t.initiative_id, COALESCE(i.name, ''), v.level, v.tier \
         FROM task_verdicts v JOIN tasks t ON t.id = v.task_id \
         LEFT JOIN initiatives i ON i.id = t.initiative_id \
         WHERE v.canary = 0 AND v.verdict = ?1 AND v.applied = 1 AND v.level <> ?2 \
           AND v.created_at >= ?3 AND t.status = 'complete' \
           AND NOT EXISTS (SELECT 1 FROM task_verdicts w WHERE w.task_id = v.task_id AND w.canary = 0 \
                           AND w.applied = 1 AND w.verdict = ?1 AND w.created_at > v.created_at)",
    )?;
    let mut groups: BTreeMap<String, CompletedGroup> = BTreeMap::new();
    let rows = stmt.query_map(params![VERDICT_PASS, LEVEL_OWNER, since], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (initiative_id, name, level, tier) = row?;
        let g = groups
            .entry(initiative_id.clone())
            .or_insert_with(|| CompletedGroup {
                initiative_id,
                initiative: name,
                count: 0,
                by_level: BTreeMap::new(),
                tier2: 0,
            });
        g.count += 1;
        *g.by_level.entry(level).or_insert(0) += 1;
        g.tier2 += i64::from(tier >= 2);
    }
    let mut completed: Vec<CompletedGroup> = groups.into_values().collect();
    completed.sort_by(|a, b| b.count.cmp(&a.count).then(a.initiative.cmp(&b.initiative)));
    let completed_total = completed.iter().map(|g| g.count).sum();

    // Failed and sent back to their worker since then.
    let mut stmt = conn.prepare(
        "SELECT v.task_id, COUNT(*), MAX(v.created_at) FROM task_verdicts v \
         WHERE v.canary = 0 AND v.waiting_for = ?1 AND v.created_at >= ?2 GROUP BY v.task_id \
         ORDER BY MAX(v.created_at) DESC",
    )?;
    let reworked_rows: Vec<(String, i64)> = stmt
        .query_map(params![WAIT_WORKER, since], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let reworked_total = reworked_rows.len() as i64;
    let mut reworked = Vec::new();
    for (task_id, returns) in reworked_rows.into_iter().take(LISTED) {
        let Some(task) = get_task(conn, &task_id)? else {
            continue;
        };
        let finding: String = conn
            .query_row(
                "SELECT reasons FROM task_verdicts WHERE task_id = ?1 AND canary = 0 \
                 AND waiting_for = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![task_id, WAIT_WORKER],
                |r| r.get(0),
            )
            .unwrap_or_default();
        reworked.push(ReworkedTask {
            task_id,
            display_id: task.display_id,
            title: task.title,
            returns,
            last_finding: finding
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
            status: task.status,
        });
    }

    let mut veto_windows = Vec::new();
    for hold in list_holds(conn)? {
        if let Some(task) = get_task(conn, &hold.task_id)? {
            veto_windows.push(VetoWindow {
                task_id: task.id,
                display_id: task.display_id,
                title: task.title,
                release_at: hold.release_at,
            });
        }
    }

    let audit_pool: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_verdicts v JOIN tasks t ON t.id = v.task_id \
         WHERE v.canary = 0 AND v.verdict = ?1 AND v.applied = 1 AND v.level <> ?2 \
           AND t.status = 'complete' AND NOT EXISTS (SELECT 1 FROM task_audits a WHERE a.verdict_id = v.id)",
        params![VERDICT_PASS, LEVEL_OWNER],
        |r| r.get(0),
    )?;
    let open_audits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_audits WHERE answer IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(Briefing {
        since: since.to_string(),
        mode: cfg.mode,
        needs_you,
        needs_you_total,
        bulk_offer,
        completed,
        completed_total,
        reworked,
        reworked_total,
        veto_windows,
        audit_pool,
        open_audits,
        health,
    })
}

/// The briefing as the short text a notification carries. It never lists a pass by itself.
pub fn briefing_text(b: &Briefing) -> String {
    let mut lines = Vec::new();
    if b.needs_you_total > 0 {
        lines.push(format!("{} need you.", b.needs_you_total));
    } else {
        lines.push("Nothing needs you.".to_string());
    }
    if let Some(offer) = &b.bulk_offer {
        lines.push(format!("{} Run: {}", offer.text, offer.command));
    }
    if b.completed_total > 0 {
        let groups: Vec<String> = b
            .completed
            .iter()
            .take(4)
            .map(|g| {
                let name = if g.initiative.is_empty() {
                    "other"
                } else {
                    &g.initiative
                };
                format!("{} in {name}", g.count)
            })
            .collect();
        lines.push(format!(
            "HQ verified and completed {}: {}.",
            b.completed_total,
            groups.join(", ")
        ));
    }
    if b.reworked_total > 0 {
        lines.push(format!(
            "{} failed verification and went back to their workers.",
            b.reworked_total
        ));
    }
    if !b.veto_windows.is_empty() {
        lines.push(format!(
            "{} tier 2 tasks are inside their veto window (hq verify holds).",
            b.veto_windows.len()
        ));
    }
    if b.open_audits > 0 {
        lines.push(format!(
            "{} audits wait for your agree or not (hq verify audits).",
            b.open_audits
        ));
    }
    for alert in &b.health.alerts {
        lines.push(format!("Attention: {alert}."));
    }
    lines.join("\n")
}

/// Waiting counts that are not about the owner, for the queue health line.
pub fn machine_backlog(h: &QueueHealth) -> i64 {
    [WAIT_CI, WAIT_VERIFIER, WAIT_BUDGET, WAIT_WORKER]
        .iter()
        .map(|k| h.backlog.get(*k).copied().unwrap_or(0))
        .sum()
}
