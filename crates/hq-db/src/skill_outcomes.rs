//! Settling the outcome of a skill load from signals someone can observe.
//!
//! A session exiting cleanly says the process finished, not that the work was accepted. The
//! signals read here are what happened to the work afterwards:
//!
//! - **The task.** A task that went to `ready_for_review` and then to `complete` was accepted,
//!   but only when someone other than the people who worked it completed it. The actor on the
//!   completing event must be known and must not be an actor that held a lease on the task or
//!   moved it into progress or review; a task its own worker closes, or one whose completer was
//!   not recorded, is not judged. One that went back to `in_progress`, `to_do` or `blocked` was
//!   reworked. A task waiting in review has not been judged yet and gives no signal.
//! - **A sub-agent run review.** `accepted` is accepted and `partial` is reworked. `blocked`
//!   says the run could not finish, which is not a judgement on the skill, so it is ignored.
//! - **An owner's correction of a note.** Someone other than the writer accepting a version
//!   written under the task is acceptance; disputing it is rejection; superseding it is rework.
//!
//! Each signal belongs to a load only if it happened after the load. The latest one wins. The
//! outcome keeps where it came from (`outcome_source`) and a reference to the record that said
//! so (`outcome_ref`: `task:<id>`, `run:<id>`, `note:<version id>`), never a copy of any text.
//! The session's own result (`session_result`) and the post-session review (`review`) stay as
//! weaker evidence and are replaced by any of these.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::skill_invocations::OutcomeStatus;

/// A task moving to `complete` after review, or back to work.
pub const SOURCE_TASK: &str = "task_acceptance";
/// A sub-agent run's reviewed verdict.
pub const SOURCE_RUN: &str = "run_review";
/// Someone other than the writer judging a note.
pub const SOURCE_NOTE: &str = "note_review";
/// How the session ended. Weak: a clean exit proves nothing about the work.
pub const SOURCE_SESSION: &str = "session_result";
/// The post-session reviewer's opinion. Stronger than an exit, weaker than an observed outcome.
pub const SOURCE_REVIEW: &str = "review";

/// Sources that record an observed outcome. Nothing weaker may overwrite one of these.
pub const OBSERVED_SOURCES: [&str; 3] = [SOURCE_TASK, SOURCE_RUN, SOURCE_NOTE];

/// Whether `source` is an observed outcome and not an exit code or an opinion.
pub fn is_observed(source: &str) -> bool {
    OBSERVED_SOURCES.contains(&source)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Settled {
    /// Loads looked at.
    pub examined: usize,
    /// Loads whose outcome changed.
    pub updated: usize,
}

struct Signal {
    /// UTC, `YYYY-MM-DD HH:MM:SS`, like every task timestamp, so it compares as text.
    at: String,
    status: OutcomeStatus,
    source: &'static str,
    reference: String,
}

fn sqlite_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn from_rfc3339(text: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| sqlite_time(t.with_timezone(&Utc)))
}

struct Load {
    id: String,
    session_id: String,
    task_id: Option<String>,
    loaded_at: String,
    status: Option<String>,
    reference: Option<String>,
}

struct TaskEvent {
    event_type: String,
    at: String,
    /// Who made the move. `None` before actors were recorded, or when the writer said nothing.
    actor: Option<String>,
}

/// A task's lifecycle and the people who worked it.
struct TaskHistory {
    events: Vec<TaskEvent>,
    /// Lower-cased actors that held a lease on the task or moved it into progress or review.
    workers: HashSet<String>,
}

fn norm(actor: &str) -> String {
    actor.trim().to_ascii_lowercase()
}

fn task_history(conn: &Connection, task_id: &str) -> Result<TaskHistory> {
    let mut stmt = conn.prepare(
        "SELECT event_type, occurred_at, actor FROM task_events WHERE task_id = ?1 ORDER BY id",
    )?;
    let events: Vec<TaskEvent> = stmt
        .query_map([task_id], |r| {
            Ok(TaskEvent {
                event_type: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut workers: HashSet<String> = events
        .iter()
        .filter(|e| {
            matches!(
                e.event_type.as_str(),
                "entered_in_progress" | "entered_ready_for_review"
            )
        })
        .filter_map(|e| e.actor.as_deref().map(norm))
        .collect();
    let mut stmt =
        conn.prepare("SELECT DISTINCT actor FROM task_work_sessions WHERE task_id = ?1")?;
    for actor in stmt.query_map([task_id], |r| r.get::<_, String>(0))? {
        workers.insert(norm(&actor?));
    }
    Ok(TaskHistory { events, workers })
}

/// Whether `actor` completing a task counts as someone other than the people who worked it.
/// An actor nobody recorded, or the anonymous `unknown`, proves nothing either way.
fn is_independent(actor: Option<&str>, workers: &HashSet<String>) -> bool {
    let Some(actor) = actor.map(norm).filter(|a| !a.is_empty() && a != "unknown") else {
        return false;
    };
    !workers.contains(&actor)
}

/// What happened to the task after a load at `loaded_at`: the first review that followed, and
/// whether the next move after it was to `complete` (by someone other than its workers) or back
/// to work.
fn task_signal(history: &TaskHistory, task_id: &str, loaded_at: &str) -> Option<Signal> {
    let events = &history.events;
    let review = events
        .iter()
        .position(|e| e.event_type == "entered_ready_for_review" && e.at.as_str() >= loaded_at)?;
    let next = events.get(review + 1)?;
    let status = match next.event_type.as_str() {
        "entered_complete" if is_independent(next.actor.as_deref(), &history.workers) => {
            OutcomeStatus::Accepted
        }
        "entered_complete" => return None,
        "entered_in_progress" | "entered_to_do" | "entered_blocked" => OutcomeStatus::Reworked,
        _ => return None,
    };
    Some(Signal {
        at: next.at.clone(),
        status,
        source: SOURCE_TASK,
        reference: format!("task:{task_id}"),
    })
}

struct RunVerdict {
    run_id: String,
    task_id: Option<String>,
    status: OutcomeStatus,
    at: String,
}

fn run_verdicts(conn: &Connection) -> Result<Vec<RunVerdict>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, task_id, accept_status, last_activity_at FROM subagent_runs
          WHERE accept_status IN ('accepted', 'partial') AND settled_at IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        let accept: String = r.get(2)?;
        let secs: i64 = r.get(3)?;
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            accept,
            secs,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id, task_id, accept, secs) = row?;
        let Some(at) = DateTime::from_timestamp(secs, 0) else {
            continue;
        };
        out.push(RunVerdict {
            run_id,
            task_id,
            status: if accept == "accepted" {
                OutcomeStatus::Accepted
            } else {
                OutcomeStatus::Reworked
            },
            at: sqlite_time(at),
        });
    }
    Ok(out)
}

struct NoteJudgement {
    task_id: String,
    version_id: i64,
    status: OutcomeStatus,
    at: String,
}

/// Decisions on versions written under a task, by someone other than whoever wrote them.
fn note_judgements(conn: &Connection) -> Result<Vec<NoteJudgement>> {
    let mut stmt = conn.prepare(
        "SELECT p.task_id, p.id, e.state, e.at
           FROM note_review_events e
           JOIN note_provenance p ON p.id = e.provenance_id
          WHERE p.task_id IS NOT NULL AND p.redacted = 0
            AND e.state IN ('accepted', 'disputed', 'superseded')
            AND e.reviewer <> p.actor_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (task_id, version_id, state, at) = row?;
        let status = match state.as_str() {
            "accepted" => OutcomeStatus::Accepted,
            "disputed" => OutcomeStatus::Rejected,
            _ => OutcomeStatus::Reworked,
        };
        let Some(at) = from_rfc3339(&at) else {
            continue;
        };
        out.push(NoteJudgement {
            task_id,
            version_id,
            status,
            at,
        });
    }
    Ok(out)
}

/// Settle the outcome of every skill load in the last `window_days` from the signals above.
/// Safe to run as often as wanted: it recomputes from the records and writes only what changed.
pub fn settle(conn: &Connection, window_days: u32) -> Result<Settled> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, task_id, loaded_at, outcome_status, outcome_ref
           FROM skill_invocations
          WHERE loaded_at >= datetime('now', ?1)",
    )?;
    let loads: Vec<Load> = stmt
        .query_map([format!("-{window_days} days")], |r| {
            Ok(Load {
                id: r.get(0)?,
                session_id: r.get(1)?,
                task_id: r.get(2)?,
                loaded_at: r.get(3)?,
                status: r.get(4)?,
                reference: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let runs = run_verdicts(conn)?;
    let notes = note_judgements(conn)?;
    let mut events: HashMap<String, TaskHistory> = HashMap::new();
    let mut settled = Settled::default();

    for load in &loads {
        settled.examined += 1;
        let mut signals: Vec<Signal> = Vec::new();

        if let Some(task_id) = &load.task_id {
            if !events.contains_key(task_id) {
                events.insert(task_id.clone(), task_history(conn, task_id)?);
            }
            if let Some(s) = task_signal(&events[task_id], task_id, &load.loaded_at) {
                signals.push(s);
            }
            signals.extend(
                notes
                    .iter()
                    .filter(|n| &n.task_id == task_id && n.at.as_str() >= load.loaded_at.as_str())
                    .map(|n| Signal {
                        at: n.at.clone(),
                        status: n.status,
                        source: SOURCE_NOTE,
                        reference: format!("note:{}", n.version_id),
                    }),
            );
        }
        signals.extend(
            runs.iter()
                .filter(|r| {
                    r.at.as_str() >= load.loaded_at.as_str()
                        && ((r.task_id.is_some() && r.task_id == load.task_id)
                            || r.run_id == load.session_id)
                })
                .map(|r| Signal {
                    at: r.at.clone(),
                    status: r.status,
                    source: SOURCE_RUN,
                    reference: format!("run:{}", r.run_id),
                }),
        );

        // The latest observation wins; later evidence is stronger than earlier.
        let Some(latest) = signals.into_iter().max_by(|a, b| a.at.cmp(&b.at)) else {
            continue;
        };
        if load.status.as_deref() == Some(latest.status.as_str())
            && load.reference.as_deref() == Some(latest.reference.as_str())
        {
            continue;
        }
        conn.execute(
            "UPDATE skill_invocations
                SET outcome_status = ?2, outcome_ref = ?3, outcome_score = ?4,
                    outcome_source = ?5, updated_at = datetime('now')
              WHERE id = ?1",
            params![
                load.id,
                latest.status.as_str(),
                latest.reference,
                latest.status.score(),
                latest.source
            ],
        )?;
        settled.updated += 1;
    }
    Ok(settled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::skill_invocations::{Context, InvocationTrigger, log_invocation_with};
    use crate::tasks as t;

    fn db() -> Database {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            t::create_initiative(c, "in-1", "personal", None, "Work", "work", "WRK")?;
            for id in ["tk-1", "tk-2", "tk-3"] {
                t::create_task(
                    c,
                    id,
                    "in-1",
                    &t::NewTask {
                        title: "A task",
                        created_by: "test",
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    /// Moves a task as `actor`. A completion with no actor named is made by `reviewer`, a person
    /// other than anyone who worked the task: what most tests mean by a reviewed task.
    fn move_as(db: &Database, task: &str, status: &str, actor: Option<&str>) {
        let ctx = t::WriteCtx {
            actor,
            work_session_id: None,
        };
        db.with_conn(|c| {
            t::update_task_as(
                c,
                task,
                &t::TaskPatch {
                    status: Some(status.to_string()),
                    ..Default::default()
                },
                None,
                &ctx,
            )
            .map(|_| ())
        })
        .unwrap();
    }

    fn move_to(db: &Database, task: &str, status: &str) {
        move_as(
            db,
            task,
            status,
            (status == "complete").then_some("reviewer"),
        );
    }

    fn load(db: &Database, skill: &str, session: &str, task: Option<&str>) -> String {
        db.with_conn(|c| {
            log_invocation_with(
                c,
                skill,
                session,
                InvocationTrigger::LoadSkill,
                &Context {
                    task_id: task,
                    ..Default::default()
                },
            )
        })
        .unwrap()
    }

    /// (status, score, source, ref) of one load.
    type Outcome = (Option<String>, Option<f64>, Option<String>, Option<String>);

    fn outcome(db: &Database, id: &str) -> Outcome {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT outcome_status, outcome_score, outcome_source, outcome_ref
                   FROM skill_invocations WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .unwrap()
    }

    fn settle_now(db: &Database) -> Settled {
        db.with_conn(|c| settle(c, 30)).unwrap()
    }

    fn some(status: &str, score: f64, source: &str, reference: &str) -> Outcome {
        (
            Some(status.into()),
            Some(score),
            Some(source.into()),
            Some(reference.into()),
        )
    }

    #[test]
    fn a_task_accepted_after_review_makes_the_load_accepted() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");

        let settled = settle_now(&db);

        assert_eq!(
            settled,
            Settled {
                examined: 1,
                updated: 1
            }
        );
        assert_eq!(
            outcome(&db, &id),
            some("accepted", 1.0, SOURCE_TASK, "task:tk-1")
        );
    }

    #[test]
    fn a_session_that_exited_cleanly_while_its_task_was_reopened_is_not_a_success() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        // The session ended well, which is all the weak signal knows.
        db.with_conn(|c| {
            crate::skill_invocations::update_outcome_by_session(c, "s-1", 1.0, SOURCE_SESSION)
        })
        .unwrap();
        assert_eq!(outcome(&db, &id).1, Some(1.0));
        // The work went to review and came back.
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "in_progress");

        settle_now(&db);

        assert_eq!(
            outcome(&db, &id),
            some("reworked", 0.5, SOURCE_TASK, "task:tk-1")
        );
        let mean = db
            .with_conn(|c| crate::skill_invocations::mean_outcome(c, "sheets"))
            .unwrap();
        assert_eq!(
            mean,
            Some(0.5),
            "the clean exit no longer counts as a success"
        );
    }

    #[test]
    fn a_task_still_waiting_in_review_has_not_been_judged() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        db.with_conn(|c| {
            crate::skill_invocations::update_outcome_by_session(c, "s-1", 1.0, SOURCE_SESSION)
        })
        .unwrap();
        move_to(&db, "tk-1", "ready_for_review");

        assert_eq!(settle_now(&db).updated, 0);
        assert_eq!(outcome(&db, &id).2.as_deref(), Some(SOURCE_SESSION));
    }

    #[test]
    fn finishing_a_task_without_review_is_not_acceptance() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "complete");
        settle_now(&db);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn what_happened_before_a_load_does_not_count_for_it() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");
        db.with_conn(|c| {
            c.execute(
                "UPDATE task_events SET occurred_at = datetime('now', '-2 hours')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        settle_now(&db);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn each_attempt_gets_its_own_verdict() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let first = load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "in_progress");
        // The first attempt was a while ago; the second loads after the reopen.
        db.with_conn(|c| {
            c.execute(
                "UPDATE task_events SET occurred_at = datetime('now', '-2 hours')",
                [],
            )?;
            c.execute(
                "UPDATE skill_invocations SET loaded_at = datetime('now', '-3 hours')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let second = load(&db, "sheets", "s-2", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");

        settle_now(&db);

        assert_eq!(outcome(&db, &first).0.as_deref(), Some("reworked"));
        assert_eq!(outcome(&db, &second).0.as_deref(), Some("accepted"));
    }

    fn run(db: &Database, run_id: &str, task: Option<&str>, accept: &str) {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO subagent_runs
                   (run_id, child_id, goal, task_id, started_at, last_activity_at, settled_at,
                    exec_status, accept_status)
                 VALUES (?1, 'child', 'g', ?2, strftime('%s','now'), strftime('%s','now') + 5,
                         strftime('%s','now'), 'completed', ?3)",
                rusqlite::params![run_id, task, accept],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_reviewed_run_settles_the_loads_of_its_task_and_of_its_own_session() {
        let db = db();
        let by_task = load(&db, "sheets", "parent", Some("tk-1"));
        let by_session = load(&db, "docs", "run-1", None);
        let unrelated = load(&db, "other", "elsewhere", Some("tk-2"));
        run(&db, "run-1", Some("tk-1"), "accepted");

        settle_now(&db);

        assert_eq!(
            outcome(&db, &by_task),
            some("accepted", 1.0, SOURCE_RUN, "run:run-1")
        );
        assert_eq!(
            outcome(&db, &by_session),
            some("accepted", 1.0, SOURCE_RUN, "run:run-1")
        );
        assert_eq!(outcome(&db, &unrelated), (None, None, None, None));
    }

    #[test]
    fn a_partial_run_is_rework_and_a_blocked_one_is_no_judgement() {
        let db = db();
        let partial = load(&db, "sheets", "run-p", None);
        let blocked = load(&db, "docs", "run-b", None);
        run(&db, "run-p", None, "partial");
        run(&db, "run-b", None, "blocked");

        settle_now(&db);

        assert_eq!(outcome(&db, &partial).0.as_deref(), Some("reworked"));
        assert_eq!(outcome(&db, &blocked), (None, None, None, None));
    }

    fn judge_note(
        db: &Database,
        task: &str,
        writer: &str,
        reviewer: &str,
        state: crate::provenance::ReviewState,
    ) {
        use crate::provenance::{Attribution, SourceKind, record, set_review_state};
        db.with_conn(|c| {
            let rec = record(
                c,
                "Notebooks/a.md",
                &format!("h-{writer}-{reviewer}"),
                &Attribution {
                    actor_id: Some(writer),
                    source_kind: Some(SourceKind::Agent),
                    session_id: Some("s-1"),
                    task_id: Some(task),
                },
            )?;
            set_review_state(c, rec.id, state, reviewer, Some("because"))
        })
        .unwrap();
    }

    #[test]
    fn someone_other_than_the_writer_correcting_a_note_settles_the_load() {
        use crate::provenance::ReviewState;
        let db = db();
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        judge_note(&db, "tk-1", "relay", "owner", ReviewState::Disputed);

        settle_now(&db);

        let (status, score, source, reference) = outcome(&db, &id);
        assert_eq!(
            (status.as_deref(), score, source.as_deref()),
            (Some("rejected"), Some(0.0), Some(SOURCE_NOTE))
        );
        assert!(
            reference.unwrap().starts_with("note:"),
            "a reference, not text"
        );
    }

    #[test]
    fn a_writer_judging_its_own_note_is_not_a_correction() {
        use crate::provenance::ReviewState;
        let db = db();
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        judge_note(&db, "tk-1", "relay", "relay", ReviewState::Accepted);
        settle_now(&db);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn the_latest_observation_wins() {
        use crate::provenance::ReviewState;
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");
        settle_now(&db);
        assert_eq!(outcome(&db, &id).0.as_deref(), Some("accepted"));

        // Later, the owner disputes a note the task produced: the newer evidence takes over.
        judge_note(&db, "tk-1", "relay", "owner", ReviewState::Disputed);
        db.with_conn(|c| {
            c.execute(
                "UPDATE task_events SET occurred_at = datetime('now', '-1 hours')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        settle_now(&db);
        assert_eq!(outcome(&db, &id).0.as_deref(), Some("rejected"));
    }

    #[test]
    fn settling_twice_changes_nothing_the_second_time() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");
        assert_eq!(settle_now(&db).updated, 1);
        assert_eq!(settle_now(&db).updated, 0);
    }

    #[test]
    fn a_later_opinion_or_exit_code_does_not_overwrite_an_observed_outcome() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "in_progress");
        settle_now(&db);

        db.with_conn(|c| {
            crate::skill_invocations::update_outcome_by_session(c, "s-1", 1.0, SOURCE_SESSION)?;
            crate::skill_invocations::set_latest_outcome(c, "sheets", 6, 1.0, SOURCE_REVIEW)?;
            crate::skill_invocations::set_latest_verdict(
                c,
                "sheets",
                6,
                crate::skill_invocations::Used::Helped,
                Some((1.0, SOURCE_REVIEW)),
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            outcome(&db, &id),
            some("reworked", 0.5, SOURCE_TASK, "task:tk-1")
        );
        let used: String = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT used FROM skill_invocations WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(
            used, "helped",
            "what the review said about use is still recorded"
        );
    }

    #[test]
    fn skills_whose_work_was_reopened_or_rejected_rank_below_accepted_ones() {
        let db = db();
        // accepted
        move_to(&db, "tk-1", "in_progress");
        load(&db, "good-skill", "s-1", Some("tk-1"));
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");
        // reworked
        move_to(&db, "tk-2", "in_progress");
        load(&db, "middling-skill", "s-2", Some("tk-2"));
        move_to(&db, "tk-2", "ready_for_review");
        move_to(&db, "tk-2", "in_progress");
        // rejected by a note correction
        load(&db, "bad-skill", "s-3", Some("tk-3"));
        judge_note(
            &db,
            "tk-3",
            "relay",
            "owner",
            crate::provenance::ReviewState::Disputed,
        );

        settle_now(&db);

        let mean = |name: &str| {
            db.with_conn(|c| crate::skill_invocations::mean_outcome(c, name))
                .unwrap()
                .unwrap()
        };
        assert!(mean("good-skill") > mean("middling-skill"));
        assert!(mean("middling-skill") > mean("bad-skill"));
    }

    #[test]
    fn loads_outside_the_window_are_left_alone() {
        let db = db();
        move_to(&db, "tk-1", "in_progress");
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        db.with_conn(|c| {
            c.execute(
                "UPDATE skill_invocations SET loaded_at = datetime('now', '-90 days')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        move_to(&db, "tk-1", "ready_for_review");
        move_to(&db, "tk-1", "complete");
        assert_eq!(settle_now(&db), Settled::default());
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    /// A load on a task that `worker` took through progress and into review.
    fn worked_by(db: &Database, worker: &str) -> String {
        move_as(db, "tk-1", "in_progress", Some(worker));
        let id = load(db, "sheets", "s-1", Some("tk-1"));
        move_as(db, "tk-1", "ready_for_review", Some(worker));
        id
    }

    #[test]
    fn a_task_closed_by_its_own_worker_is_not_judged() {
        let db = db();
        let id = worked_by(&db, "alpha");
        move_as(&db, "tk-1", "complete", Some("alpha"));
        assert_eq!(settle_now(&db).updated, 0);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn the_worker_name_matches_whatever_its_case_or_spacing() {
        let db = db();
        let id = worked_by(&db, "Alpha");
        move_as(&db, "tk-1", "complete", Some(" alpha "));
        settle_now(&db);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn a_completion_nobody_recorded_or_made_anonymously_is_not_judged() {
        for completer in [None, Some("unknown"), Some("  ")] {
            let db = db();
            let id = worked_by(&db, "alpha");
            move_as(&db, "tk-1", "complete", completer);
            settle_now(&db);
            assert_eq!(
                outcome(&db, &id),
                (None, None, None, None),
                "completer {completer:?} proves nothing"
            );
        }
    }

    #[test]
    fn someone_other_than_the_worker_completing_it_is_acceptance() {
        for completer in ["web", "gamma"] {
            let db = db();
            let id = worked_by(&db, "alpha");
            move_as(&db, "tk-1", "complete", Some(completer));
            settle_now(&db);
            assert_eq!(
                outcome(&db, &id),
                some("accepted", 1.0, SOURCE_TASK, "task:tk-1"),
                "completer {completer}"
            );
        }
    }

    #[test]
    fn a_lease_holder_counts_as_a_worker_even_when_another_actor_moved_the_status() {
        let db = db();
        db.with_conn(|c| {
            t::claim(
                c,
                "tk-1",
                &t::LeaseIdentity {
                    actor: "alpha",
                    ..Default::default()
                },
                900,
                false,
            )
            .map(|_| ())
        })
        .unwrap();
        let id = load(&db, "sheets", "s-1", Some("tk-1"));
        // Somebody else filed it for review; alpha, who held the lease, then closes it.
        move_as(&db, "tk-1", "ready_for_review", Some("beta"));
        move_as(&db, "tk-1", "complete", Some("alpha"));
        settle_now(&db);
        assert_eq!(outcome(&db, &id), (None, None, None, None));
    }

    #[test]
    fn rework_counts_whoever_reopens_the_task() {
        let db = db();
        let id = worked_by(&db, "alpha");
        move_as(&db, "tk-1", "in_progress", Some("alpha"));
        settle_now(&db);
        assert_eq!(
            outcome(&db, &id),
            some("reworked", 0.5, SOURCE_TASK, "task:tk-1")
        );
    }
}
