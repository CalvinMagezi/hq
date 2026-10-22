//! Verifying completion without a person reading each task (docs/plans/task-verification.md).
//!
//! Three things live here. The policy in force, kept in the database so the web server, the
//! daemon and an MCP server all meet the same rule at the one place a task becomes complete. The
//! verdict record: append-only, what a verifier decided about a task on what artifact, with the
//! facts HQ observed. And the guard on the edge into `complete`, which `apply_update` runs.

use super::*;
use hq_core::config::{VerificationConfig, VerificationMode};
use serde_json::Value;
use std::collections::HashSet;

pub const VERDICT_PASS: &str = "pass";
pub const VERDICT_FAIL: &str = "fail";
pub const VERDICT_INCONCLUSIVE: &str = "inconclusive";
pub const VERDICTS: [&str; 3] = [VERDICT_PASS, VERDICT_FAIL, VERDICT_INCONCLUSIVE];

/// Machine facts and an independent review agree.
pub const LEVEL_MACHINE_PEER: &str = "machine_peer";
/// An independent review of an artifact, without machine facts.
pub const LEVEL_PEER: &str = "peer";
/// Machine facts alone.
pub const LEVEL_MACHINE: &str = "machine";
/// The owner completed it.
pub const LEVEL_OWNER: &str = "owner";
/// Nothing evidenced it.
pub const LEVEL_NONE: &str = "none";
pub const LEVELS: [&str; 5] = [
    LEVEL_MACHINE_PEER,
    LEVEL_PEER,
    LEVEL_MACHINE,
    LEVEL_OWNER,
    LEVEL_NONE,
];

pub const INDEPENDENCE_LEVELS: [&str; 4] = [
    "none",
    "different_session",
    "different_harness",
    "different_model",
];

/// The highest tier a task can sit in.
pub const MAX_TIER: u8 = 2;

/// A completion that verification does not allow. Callers turn it into a queued review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsVerification {
    pub display_id: String,
    pub tier: u8,
}

impl std::fmt::Display for NeedsVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} cannot be completed directly: completion is verified for tier {} tasks. \
             Release it as ready_for_review; HQ completes it once it passes verification, \
             or the owner can complete it from the board.",
            self.display_id, self.tier
        )
    }
}

impl std::error::Error for NeedsVerification {}

/// The completion policy in force. A database with none, or one that cannot be read, is off.
pub fn verification_policy(conn: &Connection) -> Result<VerificationConfig> {
    let text: Option<String> = conn
        .query_row(
            "SELECT config FROM task_verification_policy WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default())
}

/// Writes a process's configuration as the policy in force. Returns whether it changed.
pub fn sync_verification_policy(conn: &Connection, cfg: &VerificationConfig) -> Result<bool> {
    let json = serde_json::to_string(cfg)?;
    let current: Option<String> = conn
        .query_row(
            "SELECT config FROM task_verification_policy WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if current.as_deref() == Some(json.as_str()) {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO task_verification_policy (id, config) VALUES (1, ?1) \
         ON CONFLICT(id) DO UPDATE SET config = excluded.config, updated_at = datetime('now')",
        params![json],
    )?;
    Ok(true)
}

/// The risk tier of a task from what is known without looking at its change: a tier 2 tag, else
/// code (tagged `code`, or linked to a commit or pull request), else the vault-only tier 0. The
/// verifier raises it further when the change touches a configured path.
pub fn task_tier(conn: &Connection, task: &Task, cfg: &VerificationConfig) -> Result<u8> {
    let tags: HashSet<String> = task
        .tags
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .collect();
    if cfg
        .tier2_tags
        .iter()
        .any(|t| tags.contains(&t.trim().to_ascii_lowercase()))
    {
        return Ok(2);
    }
    let has_code_link: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_links WHERE task_id = ?1 AND kind IN ('commit', 'pr'))",
        params![task.id],
        |r| r.get(0),
    )?;
    Ok(u8::from(tags.contains(CODE_TAG) || has_code_link))
}

/// Whether completing `task` needs a verdict under the policy in force.
pub fn completion_requires_verification(conn: &Connection, task: &Task) -> Result<bool> {
    let cfg = verification_policy(conn)?;
    if cfg.mode != VerificationMode::Enforce {
        return Ok(false);
    }
    Ok(cfg.enforces(task_tier(conn, task, &cfg)?))
}

/// Everyone who worked a task, lower-cased: whoever held a lease on it, and whoever moved it into
/// progress or review. A verifier or an acceptance must not be among them.
pub fn task_workers(conn: &Connection, task_id: &str) -> Result<HashSet<String>> {
    let mut workers = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT actor FROM task_events WHERE task_id = ?1 AND actor IS NOT NULL \
         AND event_type IN ('entered_in_progress', 'entered_ready_for_review')",
    )?;
    for actor in stmt.query_map(params![task_id], |r| r.get::<_, String>(0))? {
        workers.insert(actor?.trim().to_ascii_lowercase());
    }
    let mut stmt =
        conn.prepare("SELECT DISTINCT actor FROM task_work_sessions WHERE task_id = ?1")?;
    for actor in stmt.query_map(params![task_id], |r| r.get::<_, String>(0))? {
        workers.insert(actor?.trim().to_ascii_lowercase());
    }
    Ok(workers)
}

/// One verdict as stored.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Verdict {
    pub id: String,
    pub task_id: String,
    pub artifact: String,
    pub contract_hash: String,
    pub tier: u8,
    pub verdict: String,
    pub level: String,
    pub verifier: String,
    pub independence: String,
    pub observations: Value,
    pub criteria: Value,
    pub reasons: String,
    pub applied: bool,
    pub canary: bool,
    pub created_at: String,
    /// The submission (see [`review_marker`]) this verdict is about.
    pub submission: Option<i64>,
}

const VERDICT_COLS: &str = "id, task_id, artifact, contract_hash, tier, verdict, level, verifier, \
     independence, observations, criteria, reasons, applied, canary, created_at, submission";

fn row_to_verdict(r: &rusqlite::Row) -> rusqlite::Result<Verdict> {
    let json = |i: usize| -> rusqlite::Result<Value> {
        let text: String = r.get(i)?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    };
    Ok(Verdict {
        id: r.get(0)?,
        task_id: r.get(1)?,
        artifact: r.get(2)?,
        contract_hash: r.get(3)?,
        tier: r.get::<_, i64>(4)?.clamp(0, MAX_TIER as i64) as u8,
        verdict: r.get(5)?,
        level: r.get(6)?,
        verifier: r.get(7)?,
        independence: r.get(8)?,
        observations: json(9)?,
        criteria: json(10)?,
        reasons: r.get(11)?,
        applied: r.get::<_, i64>(12)? != 0,
        canary: r.get::<_, i64>(13)? != 0,
        created_at: r.get(14)?,
        submission: r.get(15)?,
    })
}

/// What a verifier decided. Observations are the facts HQ recorded before any model ran;
/// criteria are the per-criterion results that cite them.
#[derive(Debug, Clone)]
pub struct NewVerdict<'a> {
    pub task_id: &'a str,
    pub artifact: &'a str,
    pub contract_hash: &'a str,
    pub tier: u8,
    pub verdict: &'a str,
    pub level: &'a str,
    pub verifier: &'a str,
    pub independence: &'a str,
    pub observations: &'a Value,
    pub criteria: &'a Value,
    pub reasons: &'a str,
    /// True when the caller applies this verdict to the task in the same transaction.
    pub applied: bool,
    pub canary: bool,
    /// The submission the facts were gathered for. `None` means the task's current one; a number
    /// that is no longer current is refused, because the work was submitted again meanwhile.
    pub submission: Option<i64>,
}

/// Longest reasons text kept.
const MAX_REASONS_CHARS: usize = 2000;

fn observation_ids(observations: &Value) -> Result<HashSet<String>> {
    let Some(list) = observations.as_array() else {
        anyhow::bail!("observations must be a list of recorded facts");
    };
    let mut ids = HashSet::new();
    for item in list {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|i| !i.is_empty())
        else {
            anyhow::bail!("every observation needs an id");
        };
        if !ids.insert(id.to_string()) {
            anyhow::bail!("observation id '{id}' is used twice");
        }
    }
    Ok(ids)
}

/// A pass must rest on recorded facts: it names at least one observation, and every criterion
/// passes and cites observations that exist. Only a tier 0 task with no acceptance criteria may
/// pass with an empty list, and then only as an assessment.
fn check_pass_is_evidenced(v: &NewVerdict, ids: &HashSet<String>) -> Result<()> {
    if ids.is_empty() {
        anyhow::bail!("a pass needs at least one recorded observation");
    }
    let Some(criteria) = v.criteria.as_array() else {
        anyhow::bail!("criteria must be a list");
    };
    if criteria.is_empty() {
        if v.tier != 0 {
            anyhow::bail!(
                "a tier {} pass needs criteria, each citing an observation",
                v.tier
            );
        }
        return Ok(());
    }
    for (n, criterion) in criteria.iter().enumerate() {
        let n = n + 1;
        if criterion.get("result").and_then(Value::as_str) != Some(VERDICT_PASS) {
            anyhow::bail!("criterion {n} did not pass, so the task cannot");
        }
        let cited: Vec<&str> = criterion
            .get("observation_ids")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if cited.is_empty() {
            anyhow::bail!("criterion {n} passes without citing an observation");
        }
        if let Some(missing) = cited.iter().find(|c| !ids.contains(**c)) {
            anyhow::bail!("criterion {n} cites observation '{missing}', which was never recorded");
        }
    }
    Ok(())
}

fn insert_verdict(conn: &Connection, v: &NewVerdict) -> Result<Verdict> {
    let id = format!("vd-{}", uuid::Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO task_verdicts \
         (id, task_id, artifact, contract_hash, tier, verdict, level, verifier, independence, \
          observations, criteria, reasons, applied, canary, submission) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            v.task_id,
            v.artifact,
            v.contract_hash,
            v.tier,
            v.verdict,
            v.level,
            v.verifier,
            v.independence,
            serde_json::to_string(v.observations)?,
            serde_json::to_string(v.criteria)?,
            v.reasons
                .chars()
                .take(MAX_REASONS_CHARS)
                .collect::<String>(),
            v.applied,
            v.canary,
            v.submission,
        ],
    )?;
    conn.query_row(
        &format!("SELECT {VERDICT_COLS} FROM task_verdicts WHERE id = ?1"),
        params![id],
        row_to_verdict,
    )
    .map_err(Into::into)
}

/// Records a verifier's verdict on a task. The rules the plan states are enforced here, at the
/// one place a verdict enters, whoever the caller is: the verifier must be an identity that did
/// not work the task, and a pass must cite recorded facts.
pub fn record_verdict(conn: &Connection, v: &NewVerdict) -> Result<Verdict> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, v.task_id)?
            .ok_or_else(|| anyhow::anyhow!("task {} not found", v.task_id))?;
        if v.tier > MAX_TIER {
            anyhow::bail!("tier must be 0, 1 or 2, got {}", v.tier);
        }
        if !VERDICTS.contains(&v.verdict) {
            anyhow::bail!("verdict must be one of {}", VERDICTS.join(", "));
        }
        if !LEVELS.contains(&v.level) {
            anyhow::bail!("level must be one of {}", LEVELS.join(", "));
        }
        if !INDEPENDENCE_LEVELS.contains(&v.independence) {
            anyhow::bail!(
                "independence must be one of {}",
                INDEPENDENCE_LEVELS.join(", ")
            );
        }
        let verifier = super::leases::clean_label(v.verifier);
        if verifier.is_empty() {
            anyhow::bail!("a verdict names who made it");
        }
        if v.level == LEVEL_OWNER {
            anyhow::bail!("an owner verdict is recorded by the completion itself, not supplied");
        }
        if task_workers(conn, &task.id)?.contains(&verifier.to_ascii_lowercase()) {
            anyhow::bail!(
                "{verifier} worked {} and cannot verify it; a verifier must be independent of every actor on the task",
                task.display_id
            );
        }
        let ids = observation_ids(v.observations)?;
        if v.verdict == VERDICT_PASS {
            if v.level == LEVEL_NONE {
                anyhow::bail!("a pass names how it was evidenced; level none cannot pass");
            }
            check_pass_is_evidenced(v, &ids)?;
        }
        if v.applied && v.verdict != VERDICT_PASS {
            anyhow::bail!("only a pass can be applied to a task");
        }
        let current = review_marker(conn, &task.id)?;
        if v.submission.is_some() && v.submission != current {
            anyhow::bail!(
                "{} was submitted again since these facts were gathered; the verdict is about an earlier submission",
                task.display_id
            );
        }
        let stored = NewVerdict {
            verifier: &verifier,
            submission: current,
            ..v.clone()
        };
        insert_verdict(conn, &stored)
    })
}

/// The newest verdict on a task, if any.
pub fn latest_verdict(conn: &Connection, task_id: &str) -> Result<Option<Verdict>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {VERDICT_COLS} FROM task_verdicts WHERE task_id = ?1 \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1"
            ),
            params![task_id],
            row_to_verdict,
        )
        .optional()?)
}

/// A task's verdicts, newest first.
pub fn list_verdicts(conn: &Connection, task_id: &str, limit: usize) -> Result<Vec<Verdict>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {VERDICT_COLS} FROM task_verdicts WHERE task_id = ?1 \
         ORDER BY created_at DESC, rowid DESC LIMIT {}",
        limit.clamp(1, 200)
    ))?;
    let rows = stmt
        .query_map(params![task_id], row_to_verdict)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A verdict that may complete `task`: a pass, applied, with a level, about the submission the
/// task is in now (so one for an earlier submission cannot complete reworked work).
fn verdict_allows_completion(conn: &Connection, task: &Task, verdict_id: &str) -> Result<bool> {
    let Some(v) = conn
        .query_row(
            &format!("SELECT {VERDICT_COLS} FROM task_verdicts WHERE id = ?1"),
            params![verdict_id],
            row_to_verdict,
        )
        .optional()?
    else {
        return Ok(false);
    };
    if v.task_id != task.id || v.verdict != VERDICT_PASS || !v.applied || v.level == LEVEL_NONE {
        return Ok(false);
    }
    Ok(v.submission == review_marker(conn, &task.id)?)
}

/// How a transition into `complete` was authorised.
pub(super) enum Completion {
    /// Nothing is gated for it.
    Free,
    /// The owner completed it; the completion records an owner verdict.
    Owner { tier: u8 },
    /// A passing verdict allows it.
    Verdict,
}

/// The guard on the edge into `complete`. With verification off nothing is gated. In shadow mode
/// an owner completion is recorded but nothing is gated. When enforced for the task's tier, only
/// the owner or a passing verdict allows it; anything else is `NeedsVerification`.
pub(super) fn authorize_completion(
    conn: &Connection,
    current: &Task,
    ctx: &WriteCtx,
) -> Result<Completion> {
    let cfg = verification_policy(conn)?;
    if cfg.mode == VerificationMode::Off {
        return Ok(Completion::Free);
    }
    let tier = task_tier(conn, current, &cfg)?;
    if ctx.owner {
        return Ok(Completion::Owner { tier });
    }
    if !cfg.enforces(tier) {
        return Ok(Completion::Free);
    }
    match ctx.verdict_id {
        Some(id) if verdict_allows_completion(conn, current, id)? => Ok(Completion::Verdict),
        Some(id) => anyhow::bail!(
            "verdict {id} does not allow completing {}: it must be a passing, applied verdict for this task, about the submission it is in now",
            current.display_id
        ),
        None => Err(NeedsVerification {
            display_id: current.display_id.clone(),
            tier,
        }
        .into()),
    }
}

/// Records that the owner completed a task, so every completion says how it was verified.
pub(super) fn record_owner_completion(
    conn: &Connection,
    task_id: &str,
    tier: u8,
    owner: &str,
) -> Result<()> {
    let observations = serde_json::json!([{
        "id": "o1",
        "source": "owner",
        "fact": "completed by the owner",
    }]);
    let criteria = serde_json::json!([]);
    insert_verdict(
        conn,
        &NewVerdict {
            task_id,
            artifact: "none",
            contract_hash: "",
            tier,
            verdict: VERDICT_PASS,
            level: LEVEL_OWNER,
            verifier: owner,
            independence: "none",
            observations: &observations,
            criteria: &criteria,
            reasons: "",
            applied: true,
            canary: false,
            submission: None,
        },
    )?;
    Ok(())
}

/// Id of the task's latest move into review. It names one submission: a verdict is about the
/// submission it was made on, and is void if the task has been submitted again since.
pub fn review_marker(conn: &Connection, task_id: &str) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT MAX(id) FROM task_events WHERE task_id = ?1 AND event_type = 'entered_ready_for_review'",
        params![task_id],
        |r| r.get(0),
    )?)
}

/// A task waiting for review and the facts that decide when it is verified.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub task: Task,
    pub tier: u8,
    /// Open tasks that depend on this one.
    pub dependents: i64,
    /// When it entered review. Empty for a task from before events were kept.
    pub entered_at: String,
    /// Names the submission, as [`review_marker`] does.
    pub marker: Option<i64>,
}

/// Tasks in review that have no verdict for their current submission, in the order they should
/// be verified: those other work waits on first, then tier 2 (its veto clock should start early),
/// then the longest waiting.
pub fn verification_candidates(
    conn: &Connection,
    cfg: &VerificationConfig,
) -> Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, \
                COALESCE((SELECT MAX(occurred_at) FROM task_events e \
                          WHERE e.task_id = t.id AND e.event_type = 'entered_ready_for_review'), '') \
         FROM tasks t \
         WHERE t.status = 'ready_for_review' AND t.archived_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM task_verdicts v WHERE v.task_id = t.id AND v.canary = 0 \
                 AND v.submission IS (SELECT MAX(e.id) FROM task_events e \
                       WHERE e.task_id = t.id AND e.event_type = 'entered_ready_for_review'))",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, entered_at) in rows {
        let Some(task) = get_task(conn, &id)? else {
            continue;
        };
        let dependents: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies d JOIN tasks x ON x.id = d.task_id \
             WHERE d.depends_on_task_id = ?1 AND x.status <> 'complete' AND x.archived_at IS NULL",
            params![id],
            |r| r.get(0),
        )?;
        let tier = task_tier(conn, &task, cfg)?;
        let marker = review_marker(conn, &id)?;
        out.push(Candidate {
            task,
            tier,
            dependents,
            entered_at,
            marker,
        });
    }
    out.sort_by(|a, b| {
        (b.dependents > 0)
            .cmp(&(a.dependents > 0))
            .then((b.tier == MAX_TIER).cmp(&(a.tier == MAX_TIER)))
            .then(a.entered_at.cmp(&b.entered_at))
            .then(a.task.id.cmp(&b.task.id))
    });
    Ok(out)
}

/// Dollars the reviewer has cost in the current UTC month, from the usage ledger.
pub fn reviewer_spend_this_month(conn: &Connection) -> Result<f64> {
    use hq_core::config::{BudgetPeriod, BudgetScope};
    let now = chrono::Utc::now().timestamp();
    let (since, until) = crate::usage_ledger::period_bounds(BudgetPeriod::Month, now);
    crate::usage_ledger::scope_spend(
        conn,
        &BudgetScope::Origin(hq_llm_origin().to_string()),
        since,
        until,
    )
}

/// The origin tag reviewer calls carry in the usage ledger (`hq_llm::origin::VERIFICATION`).
fn hq_llm_origin() -> &'static str {
    "verification"
}

/// Verdicts in which a reviewer ran, not counting seeded tests: what shadow mode's sample is
/// measured against.
pub fn reviewer_runs_recorded(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM task_verdicts WHERE canary = 0 AND observations LIKE '%\"kind\":\"reviewer_run\"%'",
        [],
        |r| r.get(0),
    )?)
}
