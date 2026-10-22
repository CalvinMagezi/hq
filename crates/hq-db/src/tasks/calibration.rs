//! How far the verifier can be trusted, measured (docs/plans/task-verification.md,
//! "Calibration"): audits the owner answers, canaries the reviewer is tested with, and escapes,
//! the completed tasks that came back. Autonomy is earned from those numbers, per tier and per
//! verifier configuration, and a report says "insufficient data" with the count rather than
//! inventing a rate.

use super::verification::{LEVEL_OWNER, VERDICT_PASS, verifier_config_key};
use super::*;
use hq_core::config::{AutonomyGate, VerificationConfig};
use std::collections::HashMap;

pub const AUDIT_PUSH: &str = "push";
pub const AUDIT_PULL: &str = "pull";
pub const AUDIT_SECOND_OPINION: &str = "second_opinion";
pub const AUDIT_AGREE: &str = "agree";
pub const AUDIT_DISAGREE: &str = "disagree";

/// Escapes older than this no longer count against a tier.
const ESCAPE_WINDOW_DAYS: i64 = 30;
/// Escapes in the window past which a tier stops auto-completing, whatever the audits say.
const ESCAPE_LIMIT: i64 = 3;
/// One-sided 95% normal quantile, for the upper bound on a disagreement rate.
const Z_ONE_SIDED_95: f64 = 1.645;

/// A completed task the owner is asked to check, and what they said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Audit {
    pub id: String,
    pub task_id: String,
    pub verdict_id: String,
    pub source: String,
    pub tier: u8,
    pub verifier_config: String,
    pub selected_at: String,
    pub answer: Option<String>,
    pub note: String,
    pub answered_at: Option<String>,
    pub answered_by: Option<String>,
}

const AUDIT_COLS: &str = "id, task_id, verdict_id, source, tier, verifier_config, selected_at, \
     answer, note, answered_at, answered_by";

fn row_to_audit(r: &rusqlite::Row) -> rusqlite::Result<Audit> {
    Ok(Audit {
        id: r.get(0)?,
        task_id: r.get(1)?,
        verdict_id: r.get(2)?,
        source: r.get(3)?,
        tier: r.get::<_, i64>(4)?.clamp(0, 2) as u8,
        verifier_config: r.get(5)?,
        selected_at: r.get(6)?,
        answer: r.get(7)?,
        note: r.get(8)?,
        answered_at: r.get(9)?,
        answered_by: r.get(10)?,
    })
}

/// A small seeded generator, so a selection is reproducible in a test and unpredictable in use.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct PoolItem {
    verdict_id: String,
    task_id: String,
    tier: u8,
    verifier_config: String,
}

/// Completed tasks whose newest verified pass was made by a verifier (not the owner) and has not
/// been audited.
fn audit_pool(conn: &Connection) -> Result<Vec<PoolItem>> {
    let mut stmt = conn.prepare(
        "SELECT v.id, v.task_id, v.tier, v.verifier_config \
         FROM task_verdicts v JOIN tasks t ON t.id = v.task_id \
         WHERE v.canary = 0 AND v.verdict = ?1 AND v.applied = 1 AND v.level <> ?2 \
           AND t.status = 'complete' AND t.archived_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM task_audits a WHERE a.verdict_id = v.id) \
           AND v.id = (SELECT v2.id FROM task_verdicts v2 \
                       WHERE v2.task_id = v.task_id AND v2.canary = 0 AND v2.verdict = ?1 \
                         AND v2.applied = 1 \
                       ORDER BY v2.created_at DESC, v2.rowid DESC LIMIT 1)",
    )?;
    let rows = stmt
        .query_map(params![VERDICT_PASS, LEVEL_OWNER], |r| {
            Ok(PoolItem {
                verdict_id: r.get(0)?,
                task_id: r.get(1)?,
                tier: r.get::<_, i64>(2)?.clamp(0, 2) as u8,
                verifier_config: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `(tier, config)` pairs with how many audits they have answered, and how many disagreements
/// the last month holds.
fn stratum_counts(conn: &Connection) -> Result<HashMap<(u8, String), (i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT tier, verifier_config, COUNT(*), \
                SUM(CASE WHEN answer = 'disagree' AND answered_at >= datetime('now', '-30 days') \
                         THEN 1 ELSE 0 END) \
         FROM task_audits WHERE answer IS NOT NULL GROUP BY tier, verifier_config",
    )?;
    let mut out = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            (
                r.get::<_, i64>(0)?.clamp(0, 2) as u8,
                r.get::<_, String>(1)?,
            ),
            (
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            ),
        ))
    })?;
    for row in rows {
        let (k, v) = row?;
        out.insert(k, v);
    }
    Ok(out)
}

/// Picks up to `count` completed tasks to audit, favouring the tiers and verifier configurations
/// with the least audit data, the higher tiers, and any stratum with recent disagreements. Each
/// pick is recorded as an open audit of `source`. `seed` makes the draw reproducible.
pub fn select_audits(
    conn: &Connection,
    count: usize,
    source: &str,
    seed: u64,
) -> Result<Vec<Audit>> {
    if ![AUDIT_PUSH, AUDIT_PULL].contains(&source) {
        anyhow::bail!("an audit is selected as push or pull");
    }
    in_write_tx(conn, |conn| {
        let mut pool = audit_pool(conn)?;
        let strata = stratum_counts(conn)?;
        let mut rng = Rng::new(seed);
        let mut chosen = Vec::new();
        while chosen.len() < count && !pool.is_empty() {
            let weights: Vec<f64> = pool
                .iter()
                .map(|p| {
                    let (answered, recent_bad) = strata
                        .get(&(p.tier, p.verifier_config.clone()))
                        .copied()
                        .unwrap_or((0, 0));
                    let chosen_here = chosen
                        .iter()
                        .filter(|c: &&PoolItem| {
                            c.tier == p.tier && c.verifier_config == p.verifier_config
                        })
                        .count() as f64;
                    (1.0 + f64::from(p.tier)) * (1.0 + 3.0 * recent_bad as f64)
                        / (1.0 + answered as f64 + chosen_here)
                })
                .collect();
            let total: f64 = weights.iter().sum();
            let mut pick = rng.next_f64() * total;
            let mut index = pool.len() - 1;
            for (i, w) in weights.iter().enumerate() {
                if pick < *w {
                    index = i;
                    break;
                }
                pick -= w;
            }
            chosen.push(pool.swap_remove(index));
        }
        let mut audits = Vec::with_capacity(chosen.len());
        for p in chosen {
            let id = format!("au-{}", uuid::Uuid::new_v4().simple());
            conn.execute(
                "INSERT INTO task_audits (id, task_id, verdict_id, source, tier, verifier_config) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    p.task_id,
                    p.verdict_id,
                    source,
                    p.tier,
                    p.verifier_config
                ],
            )?;
            audits.push(conn.query_row(
                &format!("SELECT {AUDIT_COLS} FROM task_audits WHERE id = ?1"),
                params![id],
                row_to_audit,
            )?);
        }
        Ok(audits)
    })
}

/// Audits waiting for an answer, oldest first.
pub fn open_audits(conn: &Connection, limit: usize) -> Result<Vec<Audit>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {AUDIT_COLS} FROM task_audits WHERE answer IS NULL \
         ORDER BY selected_at, rowid LIMIT {}",
        limit.clamp(1, 500)
    ))?;
    let rows = stmt
        .query_map([], row_to_audit)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Records the owner's answer to an audit. A disagreement reopens the task to `to_do` with a
/// comment saying why, so the miss is worked, not only counted.
pub fn answer_audit(
    conn: &Connection,
    audit_id: &str,
    agrees: bool,
    note: &str,
    answered_by: &str,
) -> Result<Audit> {
    in_write_tx(conn, |conn| {
        let audit: Audit = conn
            .query_row(
                &format!("SELECT {AUDIT_COLS} FROM task_audits WHERE id = ?1"),
                params![audit_id],
                row_to_audit,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("no audit '{audit_id}'"))?;
        if audit.answer.is_some() {
            anyhow::bail!("audit {audit_id} was already answered");
        }
        let note: String = note.chars().take(1000).collect();
        conn.execute(
            "UPDATE task_audits SET answer = ?2, note = ?3, answered_at = datetime('now'), \
             answered_by = ?4 WHERE id = ?1",
            params![
                audit_id,
                if agrees { AUDIT_AGREE } else { AUDIT_DISAGREE },
                note,
                answered_by
            ],
        )?;
        if !agrees && let Some(task) = get_task(conn, &audit.task_id)? {
            if task.status == STATUS_COMPLETE {
                let ctx = WriteCtx {
                    actor: Some(answered_by),
                    owner: true,
                    ..Default::default()
                };
                update_task_as(
                    conn,
                    &task.id,
                    &TaskPatch {
                        status: Some(STATUS_TO_DO.to_string()),
                        ..Default::default()
                    },
                    Some(STATUS_COMPLETE),
                    &ctx,
                )?;
            }
            let body = if note.trim().is_empty() {
                "The owner checked this in an audit and does not agree it was done. It is reopened."
                    .to_string()
            } else {
                format!(
                    "The owner checked this in an audit and does not agree it was done:\n> {note}"
                )
            };
            add_comment(conn, &audit.task_id, "audit", &body, None)?;
        }
        Ok(conn.query_row(
            &format!("SELECT {AUDIT_COLS} FROM task_audits WHERE id = ?1"),
            params![audit_id],
            row_to_audit,
        )?)
    })
}

/// Records that a fresh verifier, asked for a second opinion, did or did not agree with the
/// verdict. Only a disagreement is stored: an agreement adds nothing the audits do not already
/// say, and a second opinion that disagrees is a datum against the first verifier.
pub fn record_second_opinion_disagreement(
    conn: &Connection,
    verdict_id: &str,
    note: &str,
) -> Result<Audit> {
    let (task_id, tier, config): (String, i64, String) = conn
        .query_row(
            "SELECT task_id, tier, verifier_config FROM task_verdicts WHERE id = ?1",
            params![verdict_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("no verdict '{verdict_id}'"))?;
    let id = format!("au-{}", uuid::Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO task_audits (id, task_id, verdict_id, source, tier, verifier_config, answer, \
         note, answered_at, answered_by) \
         VALUES (?1, ?2, ?3, 'second_opinion', ?4, ?5, 'disagree', ?6, datetime('now'), 'verifier')",
        params![id, task_id, verdict_id, tier, config, note.chars().take(1000).collect::<String>()],
    )?;
    Ok(conn.query_row(
        &format!("SELECT {AUDIT_COLS} FROM task_audits WHERE id = ?1"),
        params![id],
        row_to_audit,
    )?)
}

/// How many audits the week's floor asks for: the larger of the minimum and the share of what
/// was auto-completed in the last seven days, and raised while a tier has recent disagreements.
pub fn weekly_audit_target(conn: &Connection, cfg: &VerificationConfig) -> Result<i64> {
    let completed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_verdicts WHERE canary = 0 AND verdict = ?1 AND applied = 1 \
         AND level <> ?2 AND created_at >= datetime('now', '-7 days') \
         AND EXISTS (SELECT 1 FROM tasks t WHERE t.id = task_id AND t.status = 'complete')",
        params![VERDICT_PASS, LEVEL_OWNER],
        |r| r.get(0),
    )?;
    let recent_bad: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_audits WHERE answer = 'disagree' \
         AND answered_at >= datetime('now', '-30 days')",
        [],
        |r| r.get(0),
    )?;
    let boost = 1.0 + recent_bad.min(4) as f64;
    let share = (cfg.audit_share() * boost).min(1.0);
    let by_share = (share * completed as f64).ceil() as i64;
    Ok(by_share.max(i64::from(cfg.audit_min_per_week)))
}

/// Selects the audits still owed for this week's floor (pushed ones count), and returns the new
/// ones. Safe to call as often as wanted: a week that has its floor selects nothing.
pub fn push_weekly_audits(
    conn: &Connection,
    cfg: &VerificationConfig,
    seed: u64,
) -> Result<Vec<Audit>> {
    let target = weekly_audit_target(conn, cfg)?;
    let pushed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_audits WHERE source = 'push' \
         AND selected_at >= datetime('now', '-7 days')",
        [],
        |r| r.get(0),
    )?;
    let owed = (target - pushed).max(0) as usize;
    if owed == 0 {
        return Ok(Vec::new());
    }
    select_audits(conn, owed, AUDIT_PUSH, seed)
}

/// One canary run, as the tools record it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanaryRun {
    pub kind: String,
    pub expected: String,
    pub got: String,
    pub correct: bool,
    pub verifier_config: String,
    pub detail: String,
}

pub fn record_canary(
    conn: &Connection,
    kind: &str,
    expected: &str,
    got: &str,
    verifier_config: &str,
    detail: &str,
) -> Result<CanaryRun> {
    // A defect is caught by anything but a pass; a control must be passed. An error says nothing
    // about the reviewer's judgement and is kept apart from both rates.
    let correct = match (expected, got) {
        ("fail", "pass") | ("pass", "fail" | "unclear") => false,
        ("fail", "fail" | "unclear") | ("pass", "pass") => true,
        _ => false,
    };
    conn.execute(
        "INSERT INTO task_canary_runs (kind, expected, got, correct, verifier_config, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            kind,
            expected,
            got,
            correct,
            verifier_config,
            detail.chars().take(500).collect::<String>()
        ],
    )?;
    Ok(CanaryRun {
        kind: kind.to_string(),
        expected: expected.to_string(),
        got: got.to_string(),
        correct,
        verifier_config: verifier_config.to_string(),
        detail: detail.to_string(),
    })
}

/// What canaries say about one verifier configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanaryStats {
    pub runs: i64,
    /// Planted defects the reviewer did not pass, of those planted.
    pub defects: i64,
    pub caught: i64,
    /// Clean controls it passed, of those given.
    pub controls: i64,
    pub passed: i64,
    pub catch_rate: Option<f64>,
    pub false_reject_rate: Option<f64>,
}

pub fn canary_stats(conn: &Connection, verifier_config: &str) -> Result<CanaryStats> {
    let (defects, caught, controls, passed): (i64, i64, i64, i64) = conn.query_row(
        "SELECT \
           COALESCE(SUM(expected = 'fail' AND got <> 'error'), 0), \
           COALESCE(SUM(expected = 'fail' AND got IN ('fail', 'unclear')), 0), \
           COALESCE(SUM(expected = 'pass' AND got <> 'error'), 0), \
           COALESCE(SUM(expected = 'pass' AND got = 'pass'), 0) \
         FROM task_canary_runs WHERE verifier_config = ?1",
        params![verifier_config],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let ratio = |n: i64, d: i64| (d > 0).then(|| n as f64 / d as f64);
    Ok(CanaryStats {
        runs: defects + controls,
        defects,
        caught,
        controls,
        passed,
        catch_rate: ratio(caught, defects),
        false_reject_rate: ratio(controls - passed, controls),
    })
}

/// Escapes in the window: completed tasks, verified by this configuration at this tier, that were
/// reopened or that have a follow-up tagged `regression` linked to them. A pass that never
/// completed the task (one a sweep recorded and has not applied, or one the owner vetoed) did not
/// escape: the task was not completed, so there is nothing to reopen.
pub fn escapes(conn: &Connection, tier: u8, verifier_config: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT v.task_id) FROM task_verdicts v JOIN tasks t ON t.id = v.task_id \
         WHERE v.canary = 0 AND v.verdict = ?1 AND v.applied = 1 AND v.level <> ?2 \
           AND v.tier = ?3 AND v.verifier_config = ?4 \
           AND v.created_at >= datetime('now', ?5) \
           AND NOT EXISTS (SELECT 1 FROM task_veto_holds h WHERE h.task_id = t.id) \
           AND ((t.status <> 'complete' \
                 AND EXISTS (SELECT 1 FROM task_events e WHERE e.task_id = t.id \
                               AND e.to_status = 'complete' AND e.occurred_at >= v.created_at)) \
                OR EXISTS (SELECT 1 FROM task_links l JOIN task_tags g \
                             ON g.task_id = l.task_id AND lower(g.tag) = 'regression' \
                           WHERE l.kind = 'task' AND l.ref = t.id) \
                OR EXISTS (SELECT 1 FROM task_links l JOIN task_tags g \
                             ON g.task_id = l.ref AND lower(g.tag) = 'regression' \
                           WHERE l.kind = 'task' AND l.task_id = t.id))",
        params![
            VERDICT_PASS,
            LEVEL_OWNER,
            tier,
            verifier_config,
            format!("-{ESCAPE_WINDOW_DAYS} days")
        ],
        |r| r.get(0),
    )?)
}

/// Upper bound of a one-sided 95% Wilson interval on a rate seen `bad` times in `n`.
pub fn wilson_upper(bad: i64, n: i64) -> f64 {
    if n <= 0 {
        return 1.0;
    }
    let n = n as f64;
    let p = bad as f64 / n;
    let z2 = Z_ONE_SIDED_95 * Z_ONE_SIDED_95;
    let centre = p + z2 / (2.0 * n);
    let spread = Z_ONE_SIDED_95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((centre + spread) / (1.0 + z2 / n)).min(1.0)
}

/// Whether a tier's auto-completion is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Enough audits and canaries, and they are good.
    Earned,
    /// Too little data to say either way.
    Insufficient,
    /// Enough data to say the verifier cannot be trusted here.
    Withdrawn,
}

/// What is known about one tier under the configuration in force.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TierCalibration {
    pub tier: u8,
    pub audits: i64,
    pub disagreements: i64,
    /// Upper bound on the disagreement rate, once there are audits.
    pub disagreement_upper: Option<f64>,
    pub escapes: i64,
    pub state: Autonomy,
    /// Why the state is what it is, in words.
    pub why: Vec<String>,
}

/// The whole picture under the configuration in force.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calibration {
    pub verifier_config: String,
    pub canaries: CanaryStats,
    pub tiers: Vec<TierCalibration>,
    pub open_audits: i64,
}

/// The configuration a verdict made now would carry.
pub fn current_config_key(cfg: &VerificationConfig) -> String {
    verifier_config_key(&cfg.reviewers(), cfg.reviewer_enabled())
}

/// Calibration for every tier under the configuration in force.
pub fn calibration(conn: &Connection, cfg: &VerificationConfig) -> Result<Calibration> {
    let key = current_config_key(cfg);
    let canaries = canary_stats(conn, &key)?;
    let mut tiers = Vec::new();
    for tier in 0..=super::verification::MAX_TIER {
        let (audits, disagreements): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(answer = 'disagree'), 0) FROM task_audits \
             WHERE answer IS NOT NULL AND tier = ?1 AND verifier_config = ?2",
            params![tier, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let escapes = escapes(conn, tier, &key)?;
        let upper = (audits > 0).then(|| wilson_upper(disagreements, audits));
        let mut why = Vec::new();
        let mut withdrawn = false;
        let mut short = false;

        let need_audits = i64::from(cfg.autonomy_min_audits);
        if audits < need_audits {
            short = true;
            why.push(format!(
                "insufficient data: {audits} of {need_audits} audits answered"
            ));
        } else if disagreements as f64 / audits as f64 > cfg.autonomy_max_disagreement {
            withdrawn = true;
            why.push(format!(
                "disagreed with in {disagreements} of {audits} audits, over the {:.0}% limit",
                cfg.autonomy_max_disagreement * 100.0
            ));
        } else if upper.is_some_and(|u| u > cfg.autonomy_max_disagreement) {
            // Within the limit as seen, but too few audits to be sure it stays there.
            short = true;
            why.push(format!(
                "insufficient data: {disagreements} of {audits} audits disagreed, but the upper bound is {:.0}% against a {:.0}% limit",
                upper.unwrap_or(1.0) * 100.0,
                cfg.autonomy_max_disagreement * 100.0
            ));
        }
        let need_canaries = i64::from(cfg.autonomy_min_canaries);
        if canaries.defects < need_canaries {
            short = true;
            why.push(format!(
                "insufficient data: {} of {need_canaries} planted defects tried",
                canaries.defects
            ));
        } else if canaries
            .catch_rate
            .is_some_and(|c| c < cfg.autonomy_min_catch_rate)
        {
            withdrawn = true;
            why.push(format!(
                "the reviewer caught {} of {} planted defects ({:.0}%, needs {:.0}%)",
                canaries.caught,
                canaries.defects,
                canaries.catch_rate.unwrap_or(0.0) * 100.0,
                cfg.autonomy_min_catch_rate * 100.0
            ));
        }
        if escapes >= ESCAPE_LIMIT {
            withdrawn = true;
            why.push(format!(
                "{escapes} completed tasks came back in {ESCAPE_WINDOW_DAYS} days"
            ));
        }
        let state = if withdrawn {
            Autonomy::Withdrawn
        } else if short {
            Autonomy::Insufficient
        } else {
            why.push("enough audits and canaries, and they are good".to_string());
            Autonomy::Earned
        };
        tiers.push(TierCalibration {
            tier,
            audits,
            disagreements,
            disagreement_upper: upper,
            escapes,
            state,
            why,
        });
    }
    let open_audits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_audits WHERE answer IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(Calibration {
        verifier_config: key,
        canaries,
        tiers,
        open_audits,
    })
}

/// Why `tier` may not auto-complete right now under the gate, or `None` when it may.
pub fn autonomy_block(
    conn: &Connection,
    cfg: &VerificationConfig,
    tier: u8,
) -> Result<Option<String>> {
    if cfg.autonomy_gate == AutonomyGate::Off {
        return Ok(None);
    }
    let cal = calibration(conn, cfg)?;
    let Some(t) = cal.tiers.iter().find(|t| t.tier == tier) else {
        return Ok(None);
    };
    let blocked = t.state == Autonomy::Withdrawn
        || (cfg.autonomy_gate == AutonomyGate::Require && t.state == Autonomy::Insufficient);
    Ok(blocked.then(|| {
        format!(
            "auto-completion for tier {tier} is held for the owner: {}",
            t.why.join("; ")
        )
    }))
}
