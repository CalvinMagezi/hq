//! Post-session skill review. One LLM call reads the session and the skills
//! loaded recently, then patches, extends or creates skills directly and the
//! operator gets one FYI in the web inbox. Modelled on Hermes's review agent:
//! skills improve on use with no approval step; every change is archived and
//! audited, and `hq skills revert` undoes any of them.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use hq_core::redact::redact_secrets;
use hq_core::types::{ChatMessage, SessionResult, ValueItem, ValueKind};
use hq_db::skill_invocations::{InvocationTrigger, RecentSkill};
use hq_tools::skill_audit::{AuditEntry, Disposition, append_audit, audit_history};
use hq_tools::skill_edit::{SkillEdit, edited_text, is_managed, write_live};
use hq_tools::skills::{
    SkillDefinition, SkillProvenance, blocks_adoption, hint_problem, list_skills, parse_skill,
    render_skill, security_flags, write_proposal_text,
};
use serde::Deserialize;

/// Provenance marker on skills this reviewer wrote.
pub const REVIEWER: &str = hq_tools::skill_edit::REVIEWER;
/// Tool calls since the last review (or the agent's own `skill_manage` call)
/// before a session is worth reviewing on its own merits.
pub const MIN_TOOL_CALLS: u32 = 10;
/// Enough when a skill was loaded recently: the owner is often reacting to its output.
pub const LIGHT_MIN_TOOL_CALLS: u32 = 2;
/// How far back a loaded skill still counts as "in play" for a review.
pub const RECENT_SKILL_HOURS: u32 = 6;
const MAX_RECENT_SKILLS: u32 = 5;
const LIGHT_REVIEW_GAP: Duration = Duration::from_secs(5 * 60);
/// `value_items.source_task` for the FYI; the value bus keeps it web-only.
pub const VALUE_SOURCE: &str = "skill_review";
/// `skill_invocations.outcome_source` for scores taken from how the session ended.
pub const OUTCOME_SOURCE_SESSION: &str = "session_result";
const OUTCOME_SOURCE_REVIEW: &str = "review";
const SCORE_GOOD: f64 = 1.0;
const SCORE_PARTIAL: f64 = 0.5;
const SCORE_BAD: f64 = 0.0;
/// Skills averaging below this are listed first as patch candidates.
const LOW_OUTCOME: f64 = 0.5;

const MAX_OPS: usize = 3;
const MAX_CHANGES_PER_SKILL_PER_DAY: usize = 3;
const MAX_CREATE_HINTS: usize = 6;
const TRANSCRIPT_LOOKBACK: usize = 40;
const MAX_MESSAGE_CHARS: usize = 600;
const MAX_LOADED_SKILL_CHARS: usize = 6_000;
const MAX_SKILL_CHARS: usize = 8_000;
const MAX_DESCRIPTION_CHARS: usize = 120;
const MAX_NAME_CHARS: usize = 64;

const SYSTEM: &str = "You maintain an AI agent's skill library: markdown procedures the agent \
loads before similar tasks. Skills improve through use, and you are how. Read the session and the \
skills loaded recently, then record what the next run should do differently.\n\n\
Be active. Most sessions that did real work should produce at least one small improvement; a \
review that changes nothing after the owner corrected something is a missed lesson. Signals, \
strongest first: the owner corrected the work or was unhappy with a delivered document, sheet or \
other output (formatting, layout, tone, structure); a loaded skill was wrong, incomplete or missing \
a step (patch it now); friction, retries or errors before something worked; a new technique that \
worked reliably; a preference the owner stated.\n\n\
Owner output preferences go in a `## Preferences` section of the skill for that domain (sheet \
column widths belong in the sheets workflow skill). Add the section if it is missing.\n\n\
Prefer, in order: patch a [managed] skill loaded recently; patch another [managed] skill that \
covers the domain; create one new class-level skill (general and reusable, with no names, ids, \
file names or dates from this session). Skills not marked [managed] are read-only: put lessons \
about them in the managed skill for that domain, or create one. Never write a skill about a single \
tool call or a one-off fact. Keep patches small: change the few lines that matter, never rewrite a \
skill. A patch's `old` must be copied exactly from the skill body and appear there once; to append, \
quote the last line of a section as `old` and repeat it at the start of `new`.\n\n\
Recently loaded skills may belong to earlier messages; change one only when this session bears \
on it.\n\n\
A skill loaded via load_skill was picked by the model because no hint matched. If the owner's \
wording should have found it, add hints: 2 to 6 lowercase phrases the owner actually used, never \
single common words.\n\n\
Reply with one JSON object and nothing else, at most 3 ops:\n\
{\"ops\":[\
{\"op\":\"patch\",\"name\":\"skill\",\"old\":\"exact body text\",\"new\":\"replacement\",\"reason\":\"one sentence for the owner\"},\
{\"op\":\"create\",\"name\":\"kebab-case-name\",\"description\":\"under 120 chars, when to use it\",\
\"hints\":[\"phrase\"],\"content\":\"markdown body\",\"reason\":\"...\"},\
{\"op\":\"hints\",\"name\":\"skill\",\"add\":[\"phrase\"],\"reason\":\"...\"}],\
\"verdicts\":{\"<each recently loaded skill>\":\"helped|hurt|unused\"}}\n\
Use an empty ops list only when there is truly nothing to learn.";

#[derive(Debug, Default, Deserialize)]
struct Reply {
    /// Parsed one by one so a malformed op cannot sink its siblings.
    #[serde(default)]
    ops: Vec<serde_json::Value>,
    #[serde(default)]
    verdicts: HashMap<String, String>,
    /// The pre-ops reply shape; only `none` is still meaningful.
    #[serde(default)]
    action: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Op {
    Patch {
        name: String,
        old: String,
        new: String,
        #[serde(default)]
        reason: String,
    },
    Create {
        name: String,
        description: String,
        #[serde(default)]
        hints: Vec<String>,
        content: String,
        #[serde(default)]
        reason: String,
    },
    Hints {
        name: String,
        add: Vec<String>,
        #[serde(default)]
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Created,
    Patched,
    HintsLearned,
}

/// A skill change the review made, for the notice and the caller's log.
#[derive(Debug, PartialEq)]
pub struct Applied {
    pub name: String,
    pub change: Change,
    pub reason: String,
    /// Staged in `_proposed/` because `governance.skills_write_approval` is on.
    pub held: bool,
    pub flags: Vec<String>,
}

fn owner_session(agent_name: &str, enabled: bool) -> bool {
    enabled && agent_name == "hq"
}

/// Only the owner's own sessions teach skills: a guest or named agent's
/// transcript must not become text in the owner's catalog.
pub fn due(agent_name: &str, enabled: bool, tool_calls_since_last: u32) -> bool {
    owner_session(agent_name, enabled) && tool_calls_since_last >= MIN_TOOL_CALLS
}

/// A short owner session is worth a look if a skill was loaded recently.
pub fn light_due(agent_name: &str, enabled: bool, tool_calls_since_last: u32) -> bool {
    owner_session(agent_name, enabled) && tool_calls_since_last >= LIGHT_MIN_TOOL_CALLS
}

// ponytail: one gap for the whole process, per-chat gaps if several owners ever share a daemon.
static LAST_LIGHT_REVIEW: Mutex<Option<Instant>> = Mutex::new(None);

fn claim_light_slot() -> bool {
    let mut last = LAST_LIGHT_REVIEW.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed() < LIGHT_REVIEW_GAP) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

/// Score for every skill a session loaded, from how the session ended.
pub fn outcome_score(result: &SessionResult) -> Option<f64> {
    match result {
        SessionResult::Complete(_) => Some(SCORE_GOOD),
        SessionResult::BudgetExhausted(_) | SessionResult::TimeLimitReached(_) => {
            Some(SCORE_PARTIAL)
        }
        SessionResult::Failed { .. } => Some(SCORE_BAD),
        SessionResult::Cancelled(_) => None,
    }
}

/// Build the router-backed LLM and run the review. `full` is [`due`]; without
/// it the review runs only when a skill was loaded recently and the light-path
/// gap has passed. Errors are the caller's to log.
pub async fn run(
    vault_path: &Path,
    session_id: &str,
    messages: &[ChatMessage],
    db: Option<&hq_db::Database>,
    full: bool,
) -> Result<Vec<Applied>> {
    let loaded = db
        .and_then(|db| {
            db.with_conn(|c| {
                hq_db::skill_invocations::recent_skills(c, RECENT_SKILL_HOURS, MAX_RECENT_SKILLS)
            })
            .ok()
        })
        .unwrap_or_default();
    if !full && (loaded.is_empty() || !claim_light_slot()) {
        return Ok(Vec::new());
    }
    let router = std::sync::Arc::new(hq_llm::router::LlmRouter::from_env())
        as std::sync::Arc<dyn hq_llm::provider::LlmProvider>;
    let llm = hq_memory::MemoryLlm::with_provider(router, "bulk".to_string());
    // An unreadable config holds every write rather than guessing it allows them.
    let write_approval =
        hq_core::config::HqConfig::load().map_or(true, |c| c.governance.skills_write_approval);
    let review = Review {
        vault_path,
        session_id,
        write_approval,
    };
    review.run(messages, &loaded, db, &llm).await
}

struct Review<'a> {
    vault_path: &'a Path,
    session_id: &'a str,
    write_approval: bool,
}

impl Review<'_> {
    fn skills_dir(&self) -> std::path::PathBuf {
        hq_core::skills_dir(self.vault_path)
    }

    async fn run(
        &self,
        messages: &[ChatMessage],
        loaded: &[RecentSkill],
        db: Option<&hq_db::Database>,
        llm: &hq_memory::MemoryLlm,
    ) -> Result<Vec<Applied>> {
        let prompt = build_prompt(&self.skills_dir(), messages, loaded, db);
        let reply = parse_reply(&llm.chat(SYSTEM, &prompt).await?)?;
        if let Some(action) = reply.action.as_deref().filter(|a| *a != "none") {
            tracing::warn!(
                action,
                "skill review used the retired single-action reply; ignored"
            );
        }
        if let Some(db) = db {
            record_verdicts(db, loaded, &reply.verdicts);
        }
        let applied: Vec<Applied> = reply
            .ops
            .into_iter()
            .take(MAX_OPS)
            .filter_map(|op| match self.apply(op) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!(%e, "skill review op skipped");
                    None
                }
            })
            .collect();
        if !applied.is_empty() {
            notify(self.vault_path, &applied);
        }
        Ok(applied)
    }

    fn apply(&self, op: serde_json::Value) -> Result<Applied> {
        match serde_json::from_value::<Op>(op)? {
            Op::Create {
                name,
                description,
                hints,
                content,
                reason,
            } => self.create(&name, &description, &hints, &content, reason),
            Op::Patch {
                name,
                old,
                new,
                reason,
            } => {
                let new = redact_secrets(&new);
                let edit = SkillEdit {
                    patch: Some((&old, &new)),
                    ..Default::default()
                };
                self.edit(&name, &edit, &new, reason, Change::Patched)
            }
            Op::Hints { name, add, reason } => {
                let edit = SkillEdit {
                    add_hints: &add,
                    ..Default::default()
                };
                self.edit(&name, &edit, &add.join("\n"), reason, Change::HintsLearned)
            }
        }
    }

    fn edit(
        &self,
        name: &str,
        edit: &SkillEdit,
        added: &str,
        reason: String,
        change: Change,
    ) -> Result<Applied> {
        check_name(name)?;
        let skills_dir = self.skills_dir();
        if !is_managed(&skills_dir, name) {
            bail!("skill {name:?} is not managed by the review; refusing to edit it");
        }
        if changes_today(&skills_dir, name) >= MAX_CHANGES_PER_SKILL_PER_DAY {
            bail!("skill {name:?} already changed {MAX_CHANGES_PER_SKILL_PER_DAY} times today");
        }
        let reason = redact_secrets(reason.trim());
        let flags = self.screen(name, added, &reason)?;
        let edited = edited_text(&skills_dir, name, edit)?;
        let held = self.commit(name, &edited.text, edited.version, &reason, &flags)?;
        Ok(Applied {
            name: name.to_string(),
            change,
            reason,
            held,
            flags,
        })
    }

    fn create(
        &self,
        name: &str,
        description: &str,
        hints: &[String],
        content: &str,
        reason: String,
    ) -> Result<Applied> {
        check_name(name)?;
        if description.trim().is_empty() || description.len() > MAX_DESCRIPTION_CHARS {
            bail!("skill description missing or over {MAX_DESCRIPTION_CHARS} chars");
        }
        if content.trim().is_empty() || content.len() > MAX_SKILL_CHARS {
            bail!("skill body missing or over {MAX_SKILL_CHARS} chars");
        }
        if parse_skill(&self.skills_dir(), name).is_some() {
            bail!("skill {name:?} already exists; refusing to overwrite on create");
        }
        let skill = new_skill(name, description, hints, content, self.session_id);
        let reason = redact_secrets(reason.trim());
        let added = format!(
            "{}\n{}\n{}",
            skill.description,
            skill.content,
            skill.hints.join("\n")
        );
        let flags = self.screen(name, &added, &reason)?;
        let held = self.commit(name, &render_skill(&skill), 1, &reason, &flags)?;
        Ok(Applied {
            name: name.to_string(),
            change: Change::Created,
            reason,
            held,
            flags,
        })
    }

    /// Flags for the audit line; text that could steer the agent or hide a payload is refused.
    fn screen(&self, name: &str, added: &str, reason: &str) -> Result<Vec<String>> {
        let flags = security_flags(added);
        if !blocks_adoption(&flags) {
            return Ok(flags);
        }
        self.audit(name, 0, added, Disposition::Rejected, reason, &flags)?;
        bail!("skill {name:?} change rejected: {}", flags.join("; "))
    }

    fn commit(
        &self,
        name: &str,
        text: &str,
        version: u32,
        reason: &str,
        flags: &[String],
    ) -> Result<bool> {
        let skills_dir = self.skills_dir();
        if self.write_approval {
            write_proposal_text(&skills_dir, name, text)?;
        } else {
            write_live(&skills_dir, name, text)?;
        }
        let disposition = if self.write_approval {
            Disposition::Held
        } else {
            Disposition::Adopted
        };
        self.audit(name, version, text, disposition, reason, flags)?;
        Ok(self.write_approval)
    }

    fn audit(
        &self,
        name: &str,
        version: u32,
        text: &str,
        d: Disposition,
        reason: &str,
        flags: &[String],
    ) -> Result<()> {
        let mut entry = AuditEntry::new(name, version, text, d);
        entry.run_id = Some(self.session_id.to_string());
        entry.reason = reason.to_string();
        entry.flags = flags.to_vec();
        append_audit(&self.skills_dir(), &entry)
    }
}

fn new_skill(
    name: &str,
    description: &str,
    hints: &[String],
    content: &str,
    session_id: &str,
) -> SkillDefinition {
    let mut kept: Vec<String> = Vec::new();
    for hint in hints.iter().map(|h| h.trim().to_lowercase()) {
        if hint_problem(&hint).is_none() && !kept.contains(&hint) && kept.len() < MAX_CREATE_HINTS {
            kept.push(hint);
        }
    }
    SkillDefinition {
        name: name.to_string(),
        description: redact_secrets(description.trim()),
        // autoLoad with no hints can never fire, and validation rejects it.
        auto_load: !kept.is_empty(),
        load_full: true,
        hints: kept,
        next_skills: Vec::new(),
        bundle_only: false,
        requires_bins: Vec::new(),
        context_need: None,
        provenance: SkillProvenance {
            minted_by: REVIEWER.to_string(),
            minted_from_run_id: Some(session_id.to_string()),
            version: 1,
        },
        content: redact_secrets(content.trim()),
    }
}

fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && !name.starts_with(['-', '_'])
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        bail!("invalid skill name {name:?}");
    }
    Ok(())
}

/// Review changes adopted today; only the review stamps a run id on its audit lines.
fn changes_today(skills_dir: &Path, name: &str) -> usize {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    audit_history(skills_dir, name)
        .iter()
        .filter(|e| {
            e.disposition == Disposition::Adopted && e.run_id.is_some() && e.ts.starts_with(&today)
        })
        .count()
}

/// Only skills that were actually loaded get a verdict, so the model cannot score arbitrary ones.
fn record_verdicts(
    db: &hq_db::Database,
    loaded: &[RecentSkill],
    verdicts: &HashMap<String, String>,
) {
    for (name, verdict) in verdicts {
        let score = match verdict.as_str() {
            "helped" => SCORE_GOOD,
            "hurt" => SCORE_BAD,
            _ => continue,
        };
        if !loaded.iter().any(|l| &l.skill_name == name) {
            continue;
        }
        let result = db.with_conn(|c| {
            hq_db::skill_invocations::set_latest_outcome(
                c,
                name,
                RECENT_SKILL_HOURS,
                score,
                OUTCOME_SOURCE_REVIEW,
            )
        });
        if let Err(e) = result {
            tracing::warn!(%e, skill = %name, "skill review: verdict not recorded");
        }
    }
}

fn build_prompt(
    skills_dir: &Path,
    messages: &[ChatMessage],
    loaded: &[RecentSkill],
    db: Option<&hq_db::Database>,
) -> String {
    let catalog = catalog(skills_dir, db);
    let bodies = loaded_bodies(skills_dir, loaded);
    format!(
        "## Skill library (weakest first)\n{}\n## Skills loaded recently\n{}\n## Session (most recent last)\n{}",
        if catalog.is_empty() {
            "(empty)\n"
        } else {
            &catalog
        },
        if bodies.is_empty() {
            "(none)\n"
        } else {
            &bodies
        },
        transcript(messages),
    )
}

fn catalog(skills_dir: &Path, db: Option<&hq_db::Database>) -> String {
    let mut rows: Vec<(bool, String)> = list_skills(skills_dir)
        .iter()
        .map(|m| {
            let (count, mean) = db.map_or((0, None), |db| usage(db, &m.name));
            let managed = is_managed(skills_dir, &m.name);
            let tag = if managed { " [managed]" } else { "" };
            let hints = if managed && !m.hints.is_empty() {
                format!(" hints: {}", m.hints.join(", "))
            } else {
                String::new()
            };
            let stats = match mean {
                Some(mean) => format!(" (loads {count}, mean outcome {mean:.2})"),
                None if count > 0 => format!(" (loads {count})"),
                None => String::new(),
            };
            let weak = mean.is_some_and(|m| m < LOW_OUTCOME);
            (
                weak,
                format!("- {}{tag}: {}{hints}{stats}\n", m.name, m.description),
            )
        })
        .collect();
    rows.sort_by_key(|(weak, _)| !weak);
    rows.into_iter().map(|(_, line)| line).collect()
}

fn usage(db: &hq_db::Database, name: &str) -> (u32, Option<f64>) {
    db.with_conn(|c| {
        Ok((
            hq_db::skill_invocations::count_invocations(c, name)?,
            hq_db::skill_invocations::mean_outcome(c, name)?,
        ))
    })
    .unwrap_or((0, None))
}

fn loaded_bodies(skills_dir: &Path, loaded: &[RecentSkill]) -> String {
    loaded
        .iter()
        .filter_map(|l| {
            let skill = parse_skill(skills_dir, &l.skill_name)?;
            let tag = if is_managed(skills_dir, &skill.name) {
                "managed"
            } else {
                "read-only"
            };
            let how = if l.trigger == InvocationTrigger::AutoLoad.as_str() {
                "auto_load: a hint matched the owner's message"
            } else {
                "load_skill: the model picked it, no hint matched"
            };
            Some(format!(
                "### {} [{tag}] ({how})\n{}\n\n",
                skill.name,
                clip(&skill.content, MAX_LOADED_SKILL_CHARS)
            ))
        })
        .collect()
}

/// Redacted before truncation, so a cut can only land inside a placeholder.
fn transcript(messages: &[ChatMessage]) -> String {
    let start = messages.len().saturating_sub(TRANSCRIPT_LOOKBACK);
    messages[start..]
        .iter()
        .map(|m| {
            let role = format!("{:?}", m.role).to_lowercase();
            let calls: Vec<&str> = m.tool_calls.iter().map(|t| t.name.as_str()).collect();
            let calls = if calls.is_empty() {
                String::new()
            } else {
                format!(" [calls: {}]", calls.join(", "))
            };
            format!(
                "{role}{calls}: {}",
                clip(&redact_secrets(&m.content), MAX_MESSAGE_CHARS)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...", &s[..s.floor_char_boundary(max)])
}

/// Models wrap JSON in prose or code fences often enough to tolerate both.
fn parse_reply(reply: &str) -> Result<Reply> {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        bail!("skill review reply has no JSON object");
    };
    if end < start {
        bail!("skill review reply has no JSON object");
    }
    Ok(serde_json::from_str(&reply[start..=end])?)
}

fn change_label(change: Change) -> &'static str {
    match change {
        Change::Created => "learned",
        Change::Patched => "improved",
        Change::HintsLearned => "hints learned",
    }
}

fn notify(vault_path: &Path, applied: &[Applied]) {
    let mut names: Vec<&str> = applied.iter().map(|a| a.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let held = applied.iter().any(|a| a.held);
    let lead = if held {
        "Skill changes held for review"
    } else {
        "Skills improved"
    };
    let title = format!("{lead}: {}", names.join(", "));
    let mut body: Vec<String> = applied
        .iter()
        .map(|a| {
            let flags = if a.flags.is_empty() {
                String::new()
            } else {
                format!(" Flags: {}.", a.flags.join("; "))
            };
            format!(
                "{} ({}): {}{flags}",
                a.name,
                change_label(a.change),
                a.reason
            )
        })
        .collect();
    if held {
        body.push(
            "governance.skills_write_approval is on: `hq skills approve|reject <name>`."
                .to_string(),
        );
    }
    body.push("Undo any change with `hq skills revert <name>`.".to_string());
    let dedup_key = format!(
        "{}:{}",
        chrono::Utc::now().format("%Y-%m-%d"),
        names.join(",")
    );
    let item = ValueItem::new(VALUE_SOURCE, ValueKind::Fyi, title, body.join("\n"))
        .with_dedup_key(&dedup_key);
    if let Err(e) = hq_db::value_items::emit_at(vault_path, &item) {
        tracing::warn!(%e, "skill review: notice failed");
    }
}

#[cfg(test)]
mod tests;
