//! Verifying one task: gather the facts, decide what they settle without a model, ask an
//! independent reviewer only when they do not, and apply a pass only if the artifact is still the
//! one that was checked.
//!
//! Four stages, each separately testable:
//!
//! 1. [`gather`] reads everything outside the task that bears on it and records it.
//! 2. [`gate`] settles what facts alone can: a contradiction fails the task, a missing artifact or
//!    unreadable evidence never passes it, and it says whether a reviewer is needed.
//! 3. [`review`] asks the reviewer to judge each criterion against the recorded observations.
//! 4. [`finish`] records the verdict, and in enforce mode completes the task, after checking that
//!    the commit, the note and the submission are still the ones that were verified.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use hq_core::config::{VerificationConfig, VerificationMode};
use hq_db::Database;
use hq_db::tasks as t;
use hq_db::tasks::Task;
use serde_json::{Value, json};

use super::artifact::{Artifact, TaskFacts, discover, repos_for_cwds};
use super::collect::{
    ChecksSummary, PrFacts, collect_checks, collect_diff, collect_note, collect_pr,
    record_diff_excerpts,
};
use super::contract::{Contract, contract_of};
use super::observation::ObservationLog;
use super::probe::{ProbePolicy, probe_url};
use super::remote::GitHubRead;
use super::reviewer::{
    Answer, Judgement, ReviewRequest, ReviewTask, Reviewer, family_of, parse_answer,
};

/// Pull requests read per task; a task with more is a task that should have been split.
const MAX_PRS: usize = 3;
/// Notes read per task.
const MAX_NOTES: usize = 5;
/// URLs probed per task.
const MAX_PROBES: usize = 3;
/// Characters of a note a reviewer is shown.
const NOTE_EXCERPT_CHARS: usize = 8_000;
/// Reviewer calls made at most, however many models are configured.
const MAX_REVIEW_CALLS: usize = 4;
/// What a task with no stated criteria is judged on, at tier 0 only.
pub const ASSESSED_CRITERION: &str = "The work described in the title and description is done";

/// What a verification reads from, and under which settings.
pub struct Env<'a> {
    pub db: &'a Database,
    pub gh: &'a dyn GitHubRead,
    pub vault_path: &'a Path,
    pub cfg: &'a VerificationConfig,
    /// `tasks.route_tags`, for mailing the agents on tasks a completion unblocks.
    pub route_tags: bool,
}

/// Who or what a waiting task is waiting on. Only `Owner` counts against the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitingFor {
    /// The agent that did the work: a fix, a link, a criterion.
    Worker,
    /// A check that has not finished.
    Ci,
    /// Capacity or a reviewer that gave no usable answer.
    Verifier,
    /// No reviewer is configured, or the ceiling is reached.
    Budget,
    /// A decision only the owner can make.
    Owner,
}

impl WaitingFor {
    pub fn as_str(self) -> &'static str {
        match self {
            WaitingFor::Worker => "worker",
            WaitingFor::Ci => "ci",
            WaitingFor::Verifier => "verifier",
            WaitingFor::Budget => "budget",
            WaitingFor::Owner => "owner",
        }
    }
}

/// What a pull request and its checks looked like when read.
#[derive(Debug, Clone)]
pub struct PrEvidence {
    pub facts: PrFacts,
    pub checks: Option<ChecksSummary>,
}

/// The artifacts as they were when read, to be compared with before a pass is applied.
#[derive(Debug, Clone, Default)]
pub struct Currency {
    /// `(repo, number, head sha)`.
    pub prs: Vec<(String, u64, Option<String>)>,
    /// `(path, content hash)`.
    pub notes: Vec<(String, String)>,
}

impl Currency {
    /// The form stored with a veto window, to compare against when it closes.
    pub fn to_json(&self) -> Value {
        json!({
            "prs": self.prs.iter().map(|(r, n, sha)| json!([r, n, sha])).collect::<Vec<_>>(),
            "notes": self.notes.iter().map(|(p, h)| json!([p, h])).collect::<Vec<_>>(),
        })
    }

    /// What a verdict was about, read back from the observations stored on it: the head commit of
    /// each pull request and the content hash of each note.
    pub fn from_observations(observations: &Value) -> Self {
        let mut out = Self::default();
        for o in observations.as_array().into_iter().flatten() {
            match o["kind"].as_str() {
                Some("pr_state") => {
                    if let (Some(repo), Some(number)) =
                        (o["data"]["repo"].as_str(), o["data"]["number"].as_u64())
                    {
                        out.prs.push((
                            repo.to_string(),
                            number,
                            o["data"]["head_sha"].as_str().map(String::from),
                        ));
                    }
                }
                Some("note_hash") => {
                    if let (Some(path), Some(hash)) = (
                        o["data"]["path"].as_str(),
                        o["data"]["content_hash"].as_str(),
                    ) {
                        out.notes.push((path.to_string(), hash.to_string()));
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn from_json(v: &Value) -> Self {
        let prs = v["prs"]
            .as_array()
            .map(|all| {
                all.iter()
                    .filter_map(|p| {
                        Some((
                            p[0].as_str()?.to_string(),
                            p[1].as_u64()?,
                            p[2].as_str().map(String::from),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let notes = v["notes"]
            .as_array()
            .map(|all| {
                all.iter()
                    .filter_map(|n| Some((n[0].as_str()?.to_string(), n[1].as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Self { prs, notes }
    }
}

/// Everything gathered about a task.
#[derive(Debug)]
pub struct Evidence {
    pub task: Task,
    pub tier: u8,
    pub artifacts: Vec<Artifact>,
    pub contract: Contract,
    pub log: ObservationLog,
    pub prs: Vec<PrEvidence>,
    /// Kinds of tamper flags raised on the change, sorted.
    pub flags: Vec<String>,
    pub currency: Currency,
    /// The submission this was gathered for.
    pub marker: Option<i64>,
    /// How anything that was not linked was found, and what could not be resolved.
    pub notes: Vec<String>,
}

impl Evidence {
    pub fn primary_artifact(&self) -> String {
        self.artifacts
            .first()
            .map(Artifact::label)
            .unwrap_or_else(|| "none".to_string())
    }
}

/// Reads everything outside the task that bears on whether it is done.
pub async fn gather(env: &Env<'_>, task: &Task) -> Result<Evidence> {
    let (facts, subtasks, marker, base_tier) = {
        let cfg = env.cfg.clone();
        let task = task.clone();
        env.db.with_conn(move |c| {
            Ok((
                TaskFacts::load(c, &task.id)?,
                t::list_subtasks(c, &task.id)?,
                t::review_marker(c, &task.id)?,
                t::task_tier(c, &task, &cfg)?,
            ))
        })?
    };
    let cwd_repos = repos_for_cwds(facts.branches.iter().map(|b| b.cwd.as_str()));
    let discovery = discover(env.gh, &facts, &cwd_repos).await;
    let contract = contract_of(&task.description);
    let mut log = ObservationLog::new();
    let mut tier = base_tier;

    log.record(
        "hq",
        "contract",
        if contract.is_assessed_only() {
            "the task states no acceptance criteria".to_string()
        } else {
            format!(
                "the task states {} acceptance criteria",
                contract.criteria.len()
            )
        },
        None,
        json!({ "criteria": contract.criteria, "hash": contract.hash }),
    );
    if !subtasks.is_empty() {
        let open: Vec<&str> = subtasks
            .iter()
            .filter(|s| s.status != t::STATUS_COMPLETE)
            .map(|s| s.display_id.as_str())
            .collect();
        log.record(
            "hq",
            "subtasks",
            if open.is_empty() {
                format!("all {} sub-tasks are complete", subtasks.len())
            } else {
                format!(
                    "{} of {} sub-tasks are not complete: {}",
                    open.len(),
                    subtasks.len(),
                    open.join(", ")
                )
            },
            Some(open.is_empty()),
            json!({ "total": subtasks.len(), "open": open }),
        );
    }

    let mut prs: Vec<PrEvidence> = Vec::new();
    let mut currency = Currency::default();
    let mut flags: BTreeSet<String> = BTreeSet::new();
    for (repo, number) in discovery.prs().take(MAX_PRS) {
        let Some(facts) = collect_pr(env.gh, &mut log, repo, number).await else {
            currency.prs.push((repo.to_string(), number, None));
            continue;
        };
        currency
            .prs
            .push((repo.to_string(), number, facts.head_sha.clone()));
        let checks = match &facts.head_sha {
            Some(sha) => {
                Some(collect_checks(env.gh, &mut log, repo, sha, facts.base_ref.as_deref()).await)
            }
            None => None,
        };
        if let Some(diff) = collect_diff(env.gh, &mut log, repo, number, &env.cfg.tier2_paths).await
        {
            record_diff_excerpts(&mut log, &diff.text);
            if diff.tier2 {
                tier = tier.max(t::MAX_TIER);
            }
            if diff.flagged {
                // A flag raises the tier: the weaker the checks may have been made, the more is needed.
                tier = tier.max(t::MAX_TIER);
                flags.extend(diff.report.flags.iter().map(|f| f.kind.clone()));
            }
        }
        prs.push(PrEvidence { facts, checks });
    }
    // A commit with no pull request still has checks on it.
    for artifact in &discovery.artifacts {
        if let Artifact::Commit {
            repo: Some(repo),
            sha,
        } = artifact
        {
            collect_checks(env.gh, &mut log, repo, sha, None).await;
        }
    }
    let mut seen_notes = 0;
    for artifact in &discovery.artifacts {
        let Artifact::Note { path } = artifact else {
            continue;
        };
        if seen_notes >= MAX_NOTES {
            break;
        }
        seen_notes += 1;
        if let Some(hash) = collect_note(env.vault_path, &mut log, path) {
            currency.notes.push((path.clone(), hash));
            if let Ok(raw) = std::fs::read_to_string(env.vault_path.join(path)) {
                let cut = raw.chars().count() > NOTE_EXCERPT_CHARS;
                log.record(
                    "vault",
                    "note_excerpt",
                    format!("the content of {path}{}", if cut { " (cut)" } else { "" }),
                    None,
                    json!({ "path": path, "excerpt": raw.chars().take(NOTE_EXCERPT_CHARS).collect::<String>(), "cut": cut }),
                );
            }
        }
    }
    for url in facts
        .links
        .iter()
        .filter(|l| l.kind == "url" && l.direction == "produced")
        .take(MAX_PROBES)
    {
        probe_url(&mut log, &url.reference, ProbePolicy::default()).await;
    }
    Ok(Evidence {
        task: task.clone(),
        tier,
        artifacts: discovery.artifacts,
        contract,
        log,
        prs,
        flags: flags.into_iter().collect(),
        currency,
        marker,
        notes: discovery.notes,
    })
}

/// What a verification came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Every criterion is shown to be met.
    Pass {
        level: &'static str,
        independence: &'static str,
        criteria: Value,
        summary: String,
    },
    /// The evidence shows it is not done.
    Fail { reasons: String, criteria: Value },
    /// It cannot be decided from what exists, and saying so is the verdict.
    Inconclusive {
        reasons: String,
        waiting_for: WaitingFor,
    },
    /// Not yet decidable for a reason that will pass: no verdict is recorded; try again later.
    Defer {
        reason: String,
        waiting_for: WaitingFor,
        retry_after_secs: u64,
    },
}

impl Resolution {
    fn inconclusive(reasons: impl Into<String>, waiting_for: WaitingFor) -> Self {
        Resolution::Inconclusive {
            reasons: reasons.into(),
            waiting_for,
        }
    }
    fn defer(reason: impl Into<String>, waiting_for: WaitingFor, retry_after_secs: u64) -> Self {
        Resolution::Defer {
            reason: reason.into(),
            waiting_for,
            retry_after_secs,
        }
    }
}

/// What the facts alone settle.
#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    Settled(Resolution),
    /// The machine evidence is in order; a reviewer must now judge the criteria. `level` is what a
    /// pass would be worth.
    NeedsReview {
        level: &'static str,
    },
}

/// Seconds to wait before looking at checks that were still running.
const CI_RETRY_SECS: u64 = 300;
/// Seconds to wait before asking again when no reviewer could be used.
const REVIEWER_RETRY_SECS: u64 = 1_800;

fn contradictions(log: &ObservationLog) -> Vec<String> {
    log.items()
        .iter()
        .filter(|o| o.ok == Some(false))
        .map(|o| o.fact.clone())
        .collect()
}

/// Settles what the facts settle, without a model. A fact that contradicts completion fails the
/// task. A task with nothing to inspect, or without the machine evidence its tier needs, is never
/// passed: it is sent back to whoever can supply it.
pub fn gate(ev: &Evidence, cfg: &VerificationConfig) -> Gate {
    let bad = contradictions(&ev.log);
    if !bad.is_empty() {
        let reasons = bad.iter().take(5).cloned().collect::<Vec<_>>().join("; ");
        return Gate::Settled(Resolution::Fail {
            reasons,
            criteria: json!([]),
        });
    }
    if ev.artifacts.is_empty() {
        let mut why = "nothing to inspect: no pull request, commit or note is linked to this task, and none could be found. Link what it produced.".to_string();
        if !ev.notes.is_empty() {
            why.push_str(&format!(" ({})", ev.notes.join("; ")));
        }
        return Gate::Settled(Resolution::inconclusive(why, WaitingFor::Worker));
    }
    if ev.tier >= 1 && ev.contract.is_assessed_only() {
        return Gate::Settled(Resolution::inconclusive(
            "this task states no acceptance criteria, so there is nothing to check its change against. Add an Acceptance section to the description.",
            WaitingFor::Worker,
        ));
    }

    let mut level = "peer";
    if ev.tier >= 1 {
        if ev.prs.is_empty() {
            return Gate::Settled(Resolution::inconclusive(
                "code was produced, but no pull request is linked or could be found, and a commit alone cannot be reviewed. Link the pull request.",
                WaitingFor::Worker,
            ));
        }
        if let Some(p) = ev.prs.iter().find(|p| !p.facts.landed_or_landable()) {
            let why = if p.facts.draft {
                "is a draft"
            } else if p.facts.mergeable == Some(false) {
                "has conflicts with its base"
            } else {
                "has not landed"
            };
            return Gate::Settled(Resolution::inconclusive(
                format!("{}#{} {why}.", p.facts.repo, p.facts.number),
                WaitingFor::Worker,
            ));
        }
        let checks: Vec<&ChecksSummary> = ev.prs.iter().filter_map(|p| p.checks.as_ref()).collect();
        if checks.iter().any(|c| c.pending || c.required_waiting) {
            return Gate::Settled(Resolution::defer(
                "checks are still running, or a required check has not reported yet",
                WaitingFor::Ci,
                CI_RETRY_SECS,
            ));
        }
        let machine_ok = !checks.is_empty() && checks.iter().all(|c| c.green);
        if machine_ok {
            level = "machine_peer";
        } else if cfg.peer_only_code && checks.iter().all(|c| c.reported == 0) {
            level = "peer";
        } else {
            return Gate::Settled(Resolution::inconclusive(
                "no check results could be read for the head commit, so nothing independent shows the code works. Add a CI check, or allow peer-only review of code with tasks.verification.peer_only_code.",
                WaitingFor::Worker,
            ));
        }
    }
    if !cfg.reviewer_enabled() {
        return Gate::Settled(Resolution::defer(
            "no reviewer is configured: set tasks.verification.reviewer_monthly_ceiling_usd to allow model review",
            WaitingFor::Budget,
            REVIEWER_RETRY_SECS,
        ));
    }
    Gate::NeedsReview { level }
}

/// The criteria a reviewer is asked to judge.
fn criteria_for(ev: &Evidence) -> Vec<String> {
    if ev.contract.is_assessed_only() {
        vec![ASSESSED_CRITERION.to_string()]
    } else {
        ev.contract.criteria.clone()
    }
}

const UNCITED_NOTE: &str = "the reviewer called it met without citing a recorded observation";

/// Merges the answers of one or more reviewers into one result per criterion: any fail fails it,
/// otherwise any unclear leaves it unclear, otherwise it passes. A pass that cites nothing, or
/// cites an observation that was never recorded, is unclear: it shows nothing.
pub(super) fn merge(
    criteria: &[String],
    answers: &[Answer],
    known_ids: &[&str],
) -> Vec<(String, Judgement, Vec<String>, String)> {
    criteria
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut result = Judgement::Pass;
            let mut ids: BTreeSet<String> = BTreeSet::new();
            let mut note = String::new();
            let mut uncited = false;
            for answer in answers {
                let c = &answer.criteria[i];
                let mut j = c.result;
                if j == Judgement::Pass
                    && (c.observation_ids.is_empty()
                        || c.observation_ids
                            .iter()
                            .any(|id| !known_ids.contains(&id.as_str())))
                {
                    j = Judgement::Unclear;
                    uncited = true;
                }
                result = match (result, j) {
                    (Judgement::Fail, _) | (_, Judgement::Fail) => Judgement::Fail,
                    (Judgement::Unclear, _) | (_, Judgement::Unclear) => Judgement::Unclear,
                    _ => Judgement::Pass,
                };
                ids.extend(
                    c.observation_ids
                        .iter()
                        .filter(|id| known_ids.contains(&id.as_str()))
                        .cloned(),
                );
                if note.is_empty() || (j != Judgement::Pass && !c.note.is_empty()) {
                    note = c.note.clone();
                }
            }
            if uncited && result == Judgement::Unclear {
                note = if note.is_empty() {
                    UNCITED_NOTE.to_string()
                } else {
                    format!("{UNCITED_NOTE} ({note})")
                };
            }
            (text.clone(), result, ids.into_iter().collect(), note)
        })
        .collect()
}

/// Asks the reviewer to judge the criteria against the recorded observations. A tier 2 task needs
/// two reviewers from different model families; if only one family is available it is left
/// undecided rather than passed on a single opinion.
pub async fn review(
    env: &Env<'_>,
    ev: &mut Evidence,
    level: &'static str,
    reviewer: &dyn Reviewer,
) -> Resolution {
    let ceiling = env.cfg.reviewer_monthly_ceiling_usd.unwrap_or(0.0);
    match env.db.with_conn(t::reviewer_spend_this_month) {
        Ok(spent) if spent >= ceiling => {
            return Resolution::defer(
                format!("the reviewer ceiling for this month (${ceiling:.2}) is used"),
                WaitingFor::Budget,
                REVIEWER_RETRY_SECS,
            );
        }
        Err(e) => {
            return Resolution::defer(
                format!("the reviewer spend could not be read ({e})"),
                WaitingFor::Verifier,
                REVIEWER_RETRY_SECS,
            );
        }
        Ok(_) => {}
    }
    let criteria = criteria_for(ev);
    let need_families = if ev.tier >= t::MAX_TIER { 2 } else { 1 };
    let request = |model: &str| ReviewRequest {
        task: ReviewTask {
            display_id: ev.task.display_id.clone(),
            title: ev.task.title.clone(),
            description: ev.task.description.clone(),
            tier: ev.tier,
        },
        criteria: criteria.clone(),
        observations: ev.log.to_json(),
        flags: ev.flags.clone(),
        model: model.to_string(),
    };

    let mut answers: Vec<Answer> = Vec::new();
    let mut families: Vec<String> = Vec::new();
    let mut models_used: Vec<String> = Vec::new();
    let mut unusable: Option<String> = None;
    for model in reviewer.models().into_iter().take(MAX_REVIEW_CALLS) {
        if families.len() >= need_families {
            break;
        }
        let reply = match reviewer.review(&request(&model)).await {
            Ok(r) => r,
            Err(e) => {
                unusable = Some(format!("{model} could not review: {e}"));
                continue;
            }
        };
        let family = family_of(&reply.model);
        if families.contains(&family) {
            // The same family again is the same opinion, not a second one.
            continue;
        }
        match parse_answer(&reply.text, &criteria) {
            Ok(answer) => {
                families.push(family);
                models_used.push(reply.model);
                answers.push(answer);
            }
            Err(why) => unusable = Some(format!("{}: {why}", reply.model)),
        }
    }
    if !models_used.is_empty() || unusable.is_some() {
        ev.log.record(
            "reviewer",
            "reviewer_run",
            format!("{} reviewer(s) answered", answers.len()),
            None,
            json!({
                "models": models_used,
                "families": families,
                "summary": answers.iter().map(|a| a.summary.clone()).collect::<Vec<_>>(),
                "problem": unusable,
            }),
        );
    }
    if answers.is_empty() {
        return Resolution::defer(
            unusable.unwrap_or_else(|| "no reviewer answered".to_string()),
            WaitingFor::Verifier,
            REVIEWER_RETRY_SECS,
        );
    }
    if families.len() < need_families {
        return Resolution::defer(
            format!(
                "a tier {} task needs reviewers from {need_families} model families, but only {} answered (configure tasks.verification.reviewer_models with models from different families)",
                ev.tier,
                families.join(", ")
            ),
            WaitingFor::Owner,
            REVIEWER_RETRY_SECS * 6,
        );
    }

    // Every flag the diff raised must be addressed by name by every reviewer.
    let mut unjustified: Vec<String> = Vec::new();
    let mut unaddressed: Vec<String> = Vec::new();
    for kind in &ev.flags {
        for answer in &answers {
            match answer.flags.iter().find(|f| &f.kind == kind) {
                None => unaddressed.push(kind.clone()),
                Some(f) if !f.justified => unjustified.push(format!("{kind}: {}", f.note)),
                Some(_) => {}
            }
        }
    }
    unaddressed.sort();
    unaddressed.dedup();
    let known: Vec<&str> = ev.log.ids();
    let merged = merge(&criteria, &answers, &known);
    let criteria_json: Vec<Value> = merged
        .iter()
        .map(|(text, result, ids, note)| {
            json!({ "text": text, "result": result.as_str(), "observation_ids": ids, "note": note })
        })
        .collect();
    let failed: Vec<String> = merged
        .iter()
        .filter(|m| m.1 == Judgement::Fail)
        .map(|m| format!("\"{}\": {}", m.0, m.3))
        .collect();
    if !failed.is_empty() || !unjustified.is_empty() {
        let mut reasons = failed;
        reasons.extend(
            unjustified
                .iter()
                .map(|u| format!("the change weakens a check without a reason: {u}")),
        );
        return Resolution::Fail {
            reasons: reasons.join("; "),
            criteria: Value::Array(criteria_json),
        };
    }
    let unclear: Vec<String> = merged
        .iter()
        .filter(|m| m.1 == Judgement::Unclear)
        .map(|m| format!("\"{}\": {}", m.0, m.3))
        .collect();
    if !unclear.is_empty() || !unaddressed.is_empty() {
        let mut reasons = unclear;
        if !unaddressed.is_empty() {
            reasons.push(format!(
                "the reviewer did not address these flags: {}",
                unaddressed.join(", ")
            ));
        }
        return Resolution::inconclusive(reasons.join("; "), WaitingFor::Worker);
    }
    let summary = answers
        .iter()
        .map(|a| a.summary.clone())
        .collect::<Vec<_>>()
        .join(" | ");
    Resolution::Pass {
        level,
        independence: if families.len() >= 2 {
            "different_model"
        } else {
            "different_session"
        },
        criteria: Value::Array(criteria_json),
        summary,
    }
}

/// What a recorded verdict did to the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// The task stays where it was: shadow mode, a tier that is not enforced, or a verdict that
    /// waits for something other than its worker.
    Nothing,
    /// A pass completed it.
    Completed,
    /// Sent back to `to_do` with the findings for its worker.
    Returned,
    /// Blocked on the owner, as one exception.
    Escalated,
    /// Tier 2: verified and approved, but the task waits out its veto window before it completes.
    Held,
}

/// What came of applying a resolution.
#[derive(Debug)]
pub enum Finished {
    Recorded {
        verdict: Box<t::Verdict>,
        moved: Moved,
        waiting_for: Option<WaitingFor>,
    },
    /// Nothing was recorded. The wait is stored, so the task is looked at again after this long.
    Deferred {
        reason: String,
        waiting_for: WaitingFor,
        retry_after_secs: u64,
    },
    /// What was verified changed before the verdict could be applied.
    Voided { reason: String },
}

/// Whether the artifacts are still the ones that were verified.
pub(super) async fn still_current(env: &Env<'_>, currency: &Currency) -> Option<String> {
    for (repo, number, sha) in &currency.prs {
        let Some(sha) = sha else { continue };
        match env
            .gh
            .api_json(&format!("repos/{repo}/pulls/{number}"))
            .await
        {
            Ok(v) => {
                let now = v.pointer("/head/sha").and_then(Value::as_str);
                if now != Some(sha.as_str()) {
                    return Some(format!(
                        "{repo}#{number} has a new head commit since it was checked"
                    ));
                }
            }
            Err(e) => return Some(format!("{repo}#{number} could not be read again ({e})")),
        }
    }
    for (path, hash) in &currency.notes {
        match std::fs::read_to_string(env.vault_path.join(path)) {
            Ok(raw) if &hq_db::search::content_hash(&raw) == hash => {}
            Ok(_) => return Some(format!("{path} was edited since it was checked")),
            Err(e) => return Some(format!("{path} could not be read again ({e})")),
        }
    }
    None
}

pub(super) fn new_verifier_id() -> String {
    format!(
        "verify:{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

/// Records the verdict and, in enforce mode for an enforced tier, acts on it: a pass completes the
/// task (only if the artifacts and the submission are unchanged since they were checked), and a
/// verdict that waits on the worker sends the task back with its findings, or to the owner once it
/// has failed `max_attempts` times. A resolution that could not decide yet records a wait instead
/// of a verdict.
pub async fn finish(env: &Env<'_>, ev: &Evidence, resolution: Resolution) -> Result<Finished> {
    finish_as(env, ev, resolution, Disposition::Act).await
}

/// What `finish` does with the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Record it and act: complete, hold, send back.
    Act,
    /// Record it and move nothing. A sweep does this, then applies all its verdicts together once
    /// the owner has read the buckets. A pass is recorded as approved (`applied`), so the sweep
    /// can complete the task with it later.
    Record,
}

pub async fn finish_as(
    env: &Env<'_>,
    ev: &Evidence,
    resolution: Resolution,
    disposition: Disposition,
) -> Result<Finished> {
    let marker = ev.marker;
    let task_id = ev.task.id.clone();
    let (verdict, level, independence, criteria, reasons, waiting_for) = match resolution {
        Resolution::Defer {
            reason,
            waiting_for,
            retry_after_secs,
        } => {
            let stored = reason.clone();
            let id = task_id.clone();
            let recorded = env.db.with_conn(move |c| {
                t::in_write_tx(c, |c| {
                    if t::review_marker(c, &id)? != marker {
                        return Ok(false);
                    }
                    t::record_wait(
                        c,
                        &id,
                        waiting_for.as_str(),
                        &stored,
                        retry_after_secs,
                        chrono::Utc::now().timestamp(),
                    )?;
                    Ok(true)
                })
            })?;
            if !recorded {
                return Ok(Finished::Voided {
                    reason: "the task was submitted again while it was being verified".to_string(),
                });
            }
            return Ok(Finished::Deferred {
                reason,
                waiting_for,
                retry_after_secs,
            });
        }
        Resolution::Pass {
            level,
            independence,
            criteria,
            summary,
        } => {
            let assessed = ev.contract.is_assessed_only();
            let reasons = if assessed {
                format!("assessed, not checked: the task states no acceptance criteria. {summary}")
            } else {
                summary
            };
            (
                t::VERDICT_PASS,
                level,
                independence,
                criteria,
                reasons,
                None,
            )
        }
        Resolution::Fail { reasons, criteria } => (
            t::VERDICT_FAIL,
            t::LEVEL_NONE,
            "none",
            criteria,
            reasons,
            Some(WaitingFor::Worker),
        ),
        Resolution::Inconclusive {
            reasons,
            waiting_for,
        } => (
            t::VERDICT_INCONCLUSIVE,
            t::LEVEL_NONE,
            "none",
            json!([]),
            reasons,
            Some(waiting_for),
        ),
    };
    let enforced = env.cfg.mode == VerificationMode::Enforce && env.cfg.enforces(ev.tier);
    // A pass completes the task only while measured trust in the verifier allows it; otherwise
    // the verdict is kept and the task is left for the owner, who is told why once.
    let held = if verdict == t::VERDICT_PASS && enforced {
        let (cfg, tier) = (env.cfg.clone(), ev.tier);
        env.db
            .with_conn(move |c| t::autonomy_block(c, &cfg, tier))
            .unwrap_or(None)
    } else {
        None
    };
    let (reasons, waiting_for) = match &held {
        Some(why) => (
            format!("{reasons}\nHeld for the owner: {why}."),
            Some(WaitingFor::Owner),
        ),
        None => (reasons, waiting_for),
    };
    let applies = verdict == t::VERDICT_PASS && enforced && held.is_none();
    // Tier 2 work is approved now and completed when its veto window closes.
    let hold_for = (applies
        && disposition == Disposition::Act
        && ev.tier >= t::MAX_TIER
        && env.cfg.veto_hours > 0)
        .then(|| i64::from(env.cfg.veto_hours) * 3600);
    let release_at = hold_for.map(|secs| chrono::Utc::now().timestamp().saturating_add(secs));
    let (reasons, waiting_for) = match release_at {
        Some(at) => (
            format!(
                "{reasons}\nTier 2: it completes at {} UTC unless the owner vetoes it first.",
                clock(at)
            ),
            Some(WaitingFor::Owner),
        ),
        None => (reasons, waiting_for),
    };
    let sends_back =
        enforced && disposition == Disposition::Act && waiting_for == Some(WaitingFor::Worker);
    if applies && let Some(reason) = still_current(env, &ev.currency).await {
        return Ok(Finished::Voided { reason });
    }

    let verifier = new_verifier_id();
    let observations = ev.log.to_json();
    let artifact = ev.primary_artifact();
    let contract_hash = ev.contract.hash.clone();
    let tier = ev.tier;
    let route_tags = env.route_tags;
    let max_attempts = env.cfg.max_attempts;
    let vault_path = env.vault_path;
    let findings = reasons.clone();
    let currency_snapshot = ev.currency.to_json();
    let verifier_config = t::verifier_config_key(
        &env.cfg.reviewers(),
        ev.log.of_kind("reviewer_run").next().is_some(),
    );
    let outcome = env.db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            // The work must still be the submission that was checked: a verdict recorded about
            // an earlier one would hide the new submission from the queue.
            if t::review_marker(c, &task_id)? != marker {
                return Ok(Err(
                    "the task was submitted again while it was being verified".to_string(),
                ));
            }
            if applies || sends_back {
                let current = t::get_task(c, &task_id)?
                    .ok_or_else(|| anyhow::anyhow!("task {task_id} vanished"))?;
                if current.status != t::STATUS_READY_FOR_REVIEW {
                    return Ok(Err(
                        "the task was changed while it was being verified".to_string()
                    ));
                }
            }
            let new = t::NewVerdict {
                task_id: &task_id,
                artifact: &artifact,
                contract_hash: &contract_hash,
                tier,
                verdict,
                level,
                verifier: &verifier,
                independence,
                observations: &observations,
                criteria: &criteria,
                reasons: &reasons,
                applied: applies,
                canary: false,
                submission: marker,
                waiting_for: waiting_for.map(WaitingFor::as_str),
                verifier_config: &verifier_config,
            };
            let recorded = t::record_verdict(c, &new)?;
            t::clear_wait(c, &task_id)?;
            if disposition == Disposition::Record {
                return Ok(Ok((recorded, Moved::Nothing, None)));
            }
            if let Some(at) = release_at {
                t::create_hold(c, &task_id, &recorded.id, at, &currency_snapshot)?;
                t::add_comment(
                    c,
                    &task_id,
                    "verifier",
                    &format!(
                        "Verified by HQ ({} evidence, tier {}, artifact {}). Because this is tier 2 it \
                         completes at {} UTC unless you veto it first: `hq verify veto {}`.",
                        recorded.level,
                        recorded.tier,
                        recorded.artifact,
                        clock(at),
                        t::get_task(c, &task_id)?.map(|t| t.display_id).unwrap_or_default()
                    ),
                    None,
                )?;
                let task = t::get_task(c, &task_id)?;
                return Ok(Ok((recorded, Moved::Held, task.map(|t| (t, Vec::new())))));
            }
            if applies {
                let ctx = t::WriteCtx {
                    actor: Some(&verifier),
                    verdict_id: Some(&recorded.id),
                    ..Default::default()
                };
                let patch = t::TaskPatch {
                    status: Some(t::STATUS_COMPLETE.to_string()),
                    ..Default::default()
                };
                let done =
                    t::update_task_as(c, &task_id, &patch, Some(t::STATUS_READY_FOR_REVIEW), &ctx)?;
                t::add_comment(c, &task_id, "verifier", &completion_note(&recorded), None)?;
                let unblocked = crate::tasks::unblocked_by_transition(
                    c,
                    Some(t::STATUS_READY_FOR_REVIEW),
                    &done,
                )?;
                return Ok(Ok((recorded, Moved::Completed, Some((done, unblocked)))));
            }
            if sends_back {
                let reworked = t::send_back(c, &task_id, &verifier, &reasons, max_attempts)?;
                let moved = match reworked {
                    t::Reworked::Returned { .. } => Moved::Returned,
                    t::Reworked::Escalated { .. } => Moved::Escalated,
                };
                // A task that now waits on the owner is the one thing the owner is told about.
                let task = (moved == Moved::Escalated)
                    .then(|| t::get_task(c, &task_id))
                    .transpose()?
                    .flatten();
                return Ok(Ok((recorded, moved, task.map(|t| (t, Vec::new())))));
            }
            Ok(Ok((recorded, Moved::Nothing, None)))
        })
    })?;
    match outcome {
        Err(reason) => Ok(Finished::Voided { reason }),
        Ok((verdict, moved, follow_up)) => {
            if let Some(why) = &held {
                crate::tasks::notify_autonomy_withdrawn(
                    vault_path.to_path_buf(),
                    ev.tier,
                    why.clone(),
                )
                .await;
            }
            match (moved, follow_up) {
                (Moved::Completed, Some((task, unblocked))) => {
                    crate::tasks::notify_unblocked(vault_path, &task, &unblocked, route_tags);
                }
                (Moved::Escalated, Some((task, _))) => {
                    crate::tasks::notify_owner_exception(vault_path.to_path_buf(), task, findings)
                        .await;
                }
                (Moved::Held, Some((task, _))) => {
                    crate::tasks::notify_veto_window(
                        vault_path.to_path_buf(),
                        task,
                        release_at.map(clock).unwrap_or_default(),
                    )
                    .await;
                }
                _ => {}
            }
            // A task returned to its worker may unblock nothing, but its worker should hear of it.
            Ok(Finished::Recorded {
                verdict: Box::new(verdict),
                moved,
                waiting_for,
            })
        }
    }
}

/// `YYYY-MM-DD HH:MM` for unix seconds.
pub(super) fn clock(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// What closing the windows that are due came to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Released {
    /// Tasks completed because their window passed without a veto.
    pub completed: usize,
    /// Tasks whose artifact changed during the window, sent back to be checked again.
    pub changed: usize,
}

/// Completes the tier 2 tasks whose veto window has closed, after checking that the pull requests
/// and notes the verdict was about are still the ones it was about. A task that changed meanwhile
/// is sent back with a verdict that says so, never completed on a stale one. A task the owner
/// already completed, vetoed or that was submitted again is left alone.
pub async fn release_due_holds(env: &Env<'_>) -> Result<Released> {
    let mut out = Released::default();
    if env.cfg.mode != VerificationMode::Enforce {
        return Ok(out);
    }
    let now = chrono::Utc::now().timestamp();
    let due = env.db.with_conn(move |c| t::due_holds(c, now))?;
    for hold in due {
        let currency = Currency::from_json(&hold.currency);
        let stale = still_current(env, &currency).await;
        let (max_attempts, route_tags, vault_path) =
            (env.cfg.max_attempts, env.route_tags, env.vault_path);
        let verifier = new_verifier_id();
        let outcome = env.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                let Some(task) = t::get_task(c, &hold.task_id)? else {
                    t::clear_hold(c, &hold.task_id)?;
                    return Ok(None);
                };
                // Completed by the owner, vetoed, or submitted again: the window is moot.
                if task.status != t::STATUS_READY_FOR_REVIEW
                    || t::review_marker(c, &task.id)? != hold.submission
                {
                    t::clear_hold(c, &task.id)?;
                    return Ok(None);
                }
                let Some(approved) = t::get_verdict(c, &hold.verdict_id)? else {
                    t::clear_hold(c, &task.id)?;
                    return Ok(None);
                };
                t::clear_hold(c, &task.id)?;
                if let Some(why) = stale {
                    let observations = json!([{
                        "id": "o1",
                        "source": "hq",
                        "kind": "veto_window",
                        "fact": format!("{why} during the veto window"),
                        "ok": false,
                    }]);
                    let reasons = format!(
                        "The verified result changed during the veto window ({why}), so it is checked again."
                    );
                    t::record_verdict(
                        c,
                        &t::NewVerdict {
                            task_id: &task.id,
                            artifact: &approved.artifact,
                            contract_hash: &approved.contract_hash,
                            tier: approved.tier,
                            verdict: t::VERDICT_INCONCLUSIVE,
                            level: t::LEVEL_NONE,
                            verifier: &verifier,
                            independence: "none",
                            observations: &observations,
                            criteria: &json!([]),
                            reasons: &reasons,
                            applied: false,
                            canary: false,
                            submission: hold.submission,
                            waiting_for: Some(WaitingFor::Worker.as_str()),
                            verifier_config: &approved.verifier_config,
                        },
                    )?;
                    t::send_back(c, &task.id, &verifier, &reasons, max_attempts)?;
                    return Ok(Some((Released::default().with_changed(), None)));
                }
                let ctx = t::WriteCtx {
                    actor: Some(&verifier),
                    verdict_id: Some(&approved.id),
                    ..Default::default()
                };
                let done = t::update_task_as(
                    c,
                    &task.id,
                    &t::TaskPatch {
                        status: Some(t::STATUS_COMPLETE.to_string()),
                        ..Default::default()
                    },
                    Some(t::STATUS_READY_FOR_REVIEW),
                    &ctx,
                )?;
                t::add_comment(
                    c,
                    &task.id,
                    "verifier",
                    &format!(
                        "The veto window closed without a veto. {}",
                        completion_note(&approved)
                    ),
                    None,
                )?;
                let unblocked = crate::tasks::unblocked_by_transition(
                    c,
                    Some(t::STATUS_READY_FOR_REVIEW),
                    &done,
                )?;
                Ok(Some((Released::default().with_completed(), Some((done, unblocked)))))
            })
        })?;
        if let Some((counts, follow_up)) = outcome {
            out.completed += counts.completed;
            out.changed += counts.changed;
            if let Some((task, unblocked)) = follow_up {
                crate::tasks::notify_unblocked(vault_path, &task, &unblocked, route_tags);
            }
        }
    }
    Ok(out)
}

impl Released {
    fn with_completed(mut self) -> Self {
        self.completed = 1;
        self
    }
    fn with_changed(mut self) -> Self {
        self.changed = 1;
        self
    }
}

/// The thread comment for a completion HQ made: what HQ did, then the reviewer's words quoted so
/// they cannot pass for a line HQ wrote.
pub(super) fn completion_note(v: &t::Verdict) -> String {
    let head = format!(
        "Verified complete by HQ: {} evidence, tier {}, artifact {}.",
        v.level, v.tier, v.artifact
    );
    let quoted: Vec<String> = v
        .reasons
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| format!("> {l}"))
        .collect();
    if quoted.is_empty() {
        head
    } else {
        format!("{head}\n{}", quoted.join("\n"))
    }
}

/// Verifies one task end to end. `allow_review` is false when a reviewer must not be called, as
/// when shadow mode has used its sample.
pub async fn verify_task(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    task: &Task,
    allow_review: bool,
) -> Result<Finished> {
    verify_task_as(env, reviewer, task, allow_review, Disposition::Act).await
}

/// `verify_task` that can record without acting.
pub async fn verify_task_as(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    task: &Task,
    allow_review: bool,
    disposition: Disposition,
) -> Result<Finished> {
    let mut ev = gather(env, task).await?;
    let resolution = match gate(&ev, env.cfg) {
        Gate::Settled(r) => r,
        Gate::NeedsReview { .. } if !allow_review => Resolution::inconclusive(
            "shadow mode: the reviewer sample is used up, so only the machine evidence was recorded",
            WaitingFor::Verifier,
        ),
        Gate::NeedsReview { level } => review(env, &mut ev, level, reviewer).await,
    };
    finish_as(env, &ev, resolution, disposition).await
}
