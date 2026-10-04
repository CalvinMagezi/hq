//! Ties a harness session to the HQ task it works on. The task is the durable
//! mission record: each lifecycle event leaves a comment on it and moves its
//! status forward, but only from a state the event expects. HQ never marks a
//! task complete; that verdict belongs to whoever reviews the work.
//!
//! Every caller writes here right after winning the registry claim for the
//! event (`claim_state_alert`, `set_status_exited_if_running`), so a daemon
//! restart or an overlapping sweep records each event once. A host that cannot
//! be reached produces no event, so an asleep laptop never moves a task.
//!
//! Comments never quote the agent's screen. Pane text is untrusted (it taints a
//! session read through `harness_session_logs`), while task comments are read
//! back as trusted input, so they point at the logs instead.

use anyhow::{Result, anyhow, bail};
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow};
use hq_db::tasks::{self as t, Task};
use rusqlite::Connection;
use serde::Serialize;

const COMMENT_AUTHOR: &str = "harness-session";

/// A lifecycle event of a session.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    Launched,
    /// An existing session was attached to the task after it started.
    Linked,
    Resumed,
    /// A person or HQ sent the session a new instruction.
    Steered,
    Blocked,
    /// A launch ended at a startup dialog, so the prompt was never typed and
    /// nothing is working on the task until someone answers it.
    BlockedAtLaunch,
    /// Herdr's `done`: the agent finished a turn, not necessarily the task.
    Finished,
    Exited,
    Stopped,
    /// The supervisor dismissed a known harmless prompt; commented once per session.
    PromptDismissed,
}

/// The linked task after an event was recorded on it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskLink {
    pub task_id: String,
    pub display_id: String,
    pub status: String,
    /// Whether this event changed the task's status.
    pub moved: bool,
}

/// Internal id of the task a new session will work on. Runs before anything
/// is launched, so a bad id never leaves an untracked workspace behind.
pub fn resolve_task(conn: &Connection, id_or_display_id: &str) -> Result<String> {
    let task = t::get_task(conn, id_or_display_id)?
        .ok_or_else(|| anyhow!("no task '{id_or_display_id}'"))?;
    if task.status == t::STATUS_COMPLETE {
        bail!(
            "task {} is complete; reopen it before launching work on it",
            task.display_id
        );
    }
    Ok(task.id)
}

/// Record `event` on the session's task. `None` when the session has no task,
/// or its `mission_id` no longer matches one.
pub fn record(
    conn: &Connection,
    session: &HarnessSessionRow,
    event: Event,
) -> Result<Option<TaskLink>> {
    let Some(task) = linked_task(conn, session)? else {
        return Ok(None);
    };
    let siblings_running = match event {
        Event::Exited | Event::BlockedAtLaunch => {
            registry::count_running_for_mission(conn, &task.id, &session.id)? > 0
        }
        _ => false,
    };
    let session_running = session.status == registry::STATUS_RUNNING;
    let moved = match target(event, siblings_running, session_running) {
        Some((from, to)) => advance(conn, &task, from, to)?,
        None => None,
    };
    if let Some(body) = comment(session, event, moved.as_ref()) {
        t::add_comment(conn, &task.id, COMMENT_AUTHOR, &body, None)?;
    }
    let status = moved
        .as_ref()
        .map_or(task.status.clone(), |m| m.status.clone());
    Ok(Some(TaskLink {
        task_id: task.id,
        display_id: task.display_id,
        status,
        moved: moved.is_some(),
    }))
}

/// Attach an existing session to a task, then record the link on the task.
/// Relinking moves the session; the old task keeps its comments.
pub fn link(conn: &Connection, session_id: &str, id_or_display_id: &str) -> Result<TaskLink> {
    let task_id = resolve_task(conn, id_or_display_id)?;
    if !registry::set_mission(conn, session_id, &task_id)? {
        bail!("no harness session '{session_id}'");
    }
    let session = registry::get(conn, session_id)?
        .ok_or_else(|| anyhow!("harness session '{session_id}' vanished while linking"))?;
    record(conn, &session, Event::Linked)?
        .ok_or_else(|| anyhow!("task {task_id} vanished while linking"))
}

fn linked_task(conn: &Connection, session: &HarnessSessionRow) -> Result<Option<Task>> {
    match session.mission_id.as_deref() {
        Some(id) => t::get_task(conn, id),
        None => Ok(None),
    }
}

const RESTARTABLE: &[&str] = &[
    t::STATUS_TO_DO,
    t::STATUS_BLOCKED,
    t::STATUS_READY_FOR_REVIEW,
];

/// The statuses an event may move a task from, and where it moves it. No row
/// lists `complete`, so a finished task is never reopened or touched. Linking a
/// session that already ended records the link without restarting the task.
fn target(
    event: Event,
    siblings_running: bool,
    session_running: bool,
) -> Option<(&'static [&'static str], &'static str)> {
    match event {
        Event::Launched | Event::Resumed | Event::Steered => {
            Some((RESTARTABLE, t::STATUS_IN_PROGRESS))
        }
        Event::Linked if session_running => Some((RESTARTABLE, t::STATUS_IN_PROGRESS)),
        Event::Linked => None,
        Event::Finished => Some((&[t::STATUS_IN_PROGRESS], t::STATUS_READY_FOR_REVIEW)),
        Event::Exited if !siblings_running => Some((&[t::STATUS_IN_PROGRESS], t::STATUS_BLOCKED)),
        Event::BlockedAtLaunch if !siblings_running => {
            Some((&[t::STATUS_IN_PROGRESS], t::STATUS_BLOCKED))
        }
        Event::Exited
        | Event::BlockedAtLaunch
        | Event::Blocked
        | Event::Stopped
        | Event::PromptDismissed => None,
    }
}

/// Move `task` to `to` if it is in one of `from`. Claim-safe on its status, so
/// a concurrent edit by a person wins rather than being overwritten.
fn advance(conn: &Connection, task: &Task, from: &[&str], to: &str) -> Result<Option<Task>> {
    if !from.contains(&task.status.as_str()) {
        return Ok(None);
    }
    let patch = t::TaskPatch {
        status: Some(to.to_string()),
        ..Default::default()
    };
    t::update_task(conn, &task.id, &patch, Some(&task.status)).map(Some)
}

fn comment(session: &HarnessSessionRow, event: Event, moved: Option<&Task>) -> Option<String> {
    let who = format!(
        "Session `{}` ({}) on {}",
        session.id, session.harness, session.host
    );
    let body = match event {
        Event::Launched => format!("{who} launched in `{}`.", session.cwd),
        Event::Linked => format!(
            "{who}, working in `{}`, was linked to this task.",
            session.cwd
        ),
        Event::Resumed => format!("{who} resumed in `{}`.", session.cwd),
        Event::Steered if moved.is_some() => {
            format!("{who} was given a new instruction, so the task is back in progress.")
        }
        Event::Steered => return None,
        Event::Blocked => format!(
            "{who} is waiting at a dialog. HQ does not answer dialogs on its own: read it with `harness_session_logs` and answer with `harness_session_send` keys."
        ),
        Event::BlockedAtLaunch if moved.is_none() => return None,
        Event::BlockedAtLaunch => format!(
            "{who} stopped at a startup dialog (such as the folder-trust prompt) before it reached its prompt, so the task's prompt was not typed and nothing is working on it. The session stays registered: read the dialog with `harness_session_logs`, answer it with `harness_session_send` keys, then send the instruction."
        ),
        Event::Finished if moved.is_some() => format!(
            "{who} finished a turn, so the task is ready for review. The agent saying it is done is not verification: check the work (`harness_session_logs`) before marking the task complete."
        ),
        Event::Finished => format!("{who} finished a turn."),
        Event::Exited if moved.is_some() => format!(
            "{who} exited. No other session is working on this task, so it is blocked until someone resumes the session (`harness_session_resume`) or reviews its last output (`harness_session_logs`)."
        ),
        Event::Exited => format!("{who} exited."),
        Event::Stopped => format!("{who} was stopped."),
        Event::PromptDismissed => format!(
            "{who} showed a known harmless prompt (the feedback survey) and HQ dismissed it with a fixed key. This is noted once per session; later dismissals are in the session's event log."
        ),
    };
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;
    use hq_db::harness_sessions_registry::{NewSession, Placement};

    fn setup() -> (Database, String) {
        let db = Database::open_memory().unwrap();
        let task_id = db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                    [],
                )?;
                let task = t::create_task(
                    c,
                    "tk-1",
                    "in-1",
                    &t::NewTask {
                        title: "durable missions",
                        created_by: "test",
                        ..Default::default()
                    },
                )?;
                Ok(task.id)
            })
            .unwrap();
        (db, task_id)
    }

    fn session(db: &Database, id: &str, mission: Option<&str>) -> HarnessSessionRow {
        db.with_conn(|c| {
            registry::insert(
                c,
                &NewSession {
                    id,
                    harness: "claude-code",
                    label: "",
                    cwd: "/repo",
                    mission_id: mission,
                    placement: Placement {
                        host: "laptop",
                        agent_name: id,
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                },
            )?;
            Ok(registry::get(c, id)?.unwrap())
        })
        .unwrap()
    }

    fn apply(db: &Database, s: &HarnessSessionRow, event: Event) -> Option<TaskLink> {
        db.with_conn(|c| record(c, s, event)).unwrap()
    }

    fn status(db: &Database, task: &str) -> String {
        db.with_conn(|c| Ok(t::get_task(c, task)?.unwrap().status))
            .unwrap()
    }

    fn comments(db: &Database, task: &str) -> Vec<String> {
        db.with_conn(|c| t::list_comments(c, task))
            .unwrap()
            .into_iter()
            .map(|c| c.body)
            .collect()
    }

    fn set_task_status(db: &Database, task: &str, to: &str) {
        let patch = t::TaskPatch {
            status: Some(to.to_string()),
            ..Default::default()
        };
        db.with_conn(|c| t::update_task(c, task, &patch, None))
            .unwrap();
    }

    #[test]
    fn resolve_task_accepts_a_display_id_and_refuses_missing_or_complete_tasks() {
        let (db, task) = setup();
        let resolved = db.with_conn(|c| resolve_task(c, "FR-001")).unwrap();
        assert_eq!(resolved, task);
        assert!(db.with_conn(|c| resolve_task(c, "FR-999")).is_err());
        set_task_status(&db, &task, t::STATUS_COMPLETE);
        let err = db.with_conn(|c| resolve_task(c, &task)).unwrap_err();
        assert!(err.to_string().contains("complete"), "{err}");
    }

    #[test]
    fn a_launch_starts_the_task_and_leaves_a_comment() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        let link = apply(&db, &s, Event::Launched).unwrap();
        assert_eq!(link.display_id, "FR-001");
        assert!(link.moved);
        assert_eq!(status(&db, &task), t::STATUS_IN_PROGRESS);
        assert!(comments(&db, &task)[0].contains("hs-1"));
    }

    #[test]
    fn a_finished_turn_asks_for_review_and_never_completes_the_task() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        apply(&db, &s, Event::Launched);
        let link = apply(&db, &s, Event::Finished).unwrap();
        assert_eq!(link.status, t::STATUS_READY_FOR_REVIEW);
        let last = comments(&db, &task).pop().unwrap();
        assert!(last.contains("not verification"), "{last}");
    }

    #[test]
    fn a_new_instruction_after_a_finished_turn_puts_the_task_back_in_progress() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        apply(&db, &s, Event::Launched);
        apply(&db, &s, Event::Finished);
        let link = apply(&db, &s, Event::Steered).unwrap();
        assert_eq!(link.status, t::STATUS_IN_PROGRESS);

        let before = comments(&db, &task).len();
        let again = apply(&db, &s, Event::Steered).unwrap();
        assert!(!again.moved);
        assert_eq!(
            comments(&db, &task).len(),
            before,
            "a steer that moves nothing is not logged"
        );
    }

    #[test]
    fn a_blocked_session_escalates_by_comment_without_moving_the_task() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        apply(&db, &s, Event::Launched);
        let link = apply(&db, &s, Event::Blocked).unwrap();
        assert!(!link.moved);
        assert_eq!(status(&db, &task), t::STATUS_IN_PROGRESS);
        let last = comments(&db, &task).pop().unwrap();
        assert!(
            last.contains("dialog") && last.contains("harness_session_logs"),
            "{last}"
        );
    }

    #[test]
    fn an_exit_blocks_the_task_only_when_no_other_session_is_on_it() {
        let (db, task) = setup();
        let a = session(&db, "hs-a", Some(&task));
        let b = session(&db, "hs-b", Some(&task));
        apply(&db, &a, Event::Launched);

        db.with_conn(|c| registry::set_status(c, "hs-a", registry::STATUS_EXITED))
            .unwrap();
        let link = apply(&db, &a, Event::Exited).unwrap();
        assert!(
            !link.moved,
            "hs-b is still running, possibly on an unreachable host"
        );
        assert_eq!(status(&db, &task), t::STATUS_IN_PROGRESS);

        db.with_conn(|c| registry::set_status(c, "hs-b", registry::STATUS_EXITED))
            .unwrap();
        let link = apply(&db, &b, Event::Exited).unwrap();
        assert!(link.moved);
        assert_eq!(status(&db, &task), t::STATUS_BLOCKED);
        let last = comments(&db, &task).pop().unwrap();
        assert!(
            last.contains("harness_session_resume") && last.contains("harness_session_logs"),
            "{last}"
        );
    }

    #[test]
    fn a_launch_stopped_at_a_dialog_blocks_the_task_unless_another_session_is_on_it() {
        let (db, task) = setup();
        let a = session(&db, "hs-a", Some(&task));
        apply(&db, &a, Event::Launched);
        let link = apply(&db, &a, Event::BlockedAtLaunch).unwrap();
        assert!(link.moved);
        assert_eq!(status(&db, &task), t::STATUS_BLOCKED);
        let last = comments(&db, &task).pop().unwrap();
        assert!(
            last.contains("startup dialog") && last.contains("nothing is working"),
            "{last}"
        );
    }

    #[test]
    fn a_dialog_at_launch_leaves_the_task_and_its_comments_alone_when_it_did_not_move() {
        let (db, task) = setup();
        let a = session(&db, "hs-a", Some(&task));
        let b = session(&db, "hs-b", Some(&task));
        apply(&db, &a, Event::Launched);
        let before = comments(&db, &task).len();
        let link = apply(&db, &b, Event::BlockedAtLaunch).unwrap();
        assert!(!link.moved, "hs-a is still running on the task");
        assert_eq!(status(&db, &task), t::STATUS_IN_PROGRESS);
        assert_eq!(
            comments(&db, &task).len(),
            before,
            "no comment claims nothing is working"
        );

        let (db, task) = setup();
        let lone = session(&db, "hs-c", Some(&task));
        let link = apply(&db, &lone, Event::BlockedAtLaunch).unwrap();
        assert!(
            !link.moved,
            "a to_do task is not blocked by a dialog it never started from"
        );
        assert!(comments(&db, &task).is_empty());
    }

    #[test]
    fn an_exit_after_review_was_requested_leaves_the_review_in_place() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        apply(&db, &s, Event::Launched);
        apply(&db, &s, Event::Finished);
        apply(&db, &s, Event::Exited);
        assert_eq!(status(&db, &task), t::STATUS_READY_FOR_REVIEW);
    }

    #[test]
    fn a_complete_task_is_never_moved() {
        let (db, task) = setup();
        let s = session(&db, "hs-1", Some(&task));
        set_task_status(&db, &task, t::STATUS_COMPLETE);
        for event in [
            Event::Resumed,
            Event::Steered,
            Event::Finished,
            Event::Exited,
        ] {
            assert!(!apply(&db, &s, event).unwrap().moved);
        }
        assert_eq!(status(&db, &task), t::STATUS_COMPLETE);
    }

    #[test]
    fn linking_a_running_session_starts_the_task_but_an_ended_one_does_not() {
        let (db, task) = setup();
        session(&db, "hs-live", None);
        let linked = db.with_conn(|c| link(c, "hs-live", "FR-001")).unwrap();
        assert!(linked.moved);
        assert_eq!(status(&db, &task), t::STATUS_IN_PROGRESS);
        assert!(comments(&db, &task)[0].contains("linked"));
        let row = db
            .with_conn(|c| registry::get(c, "hs-live"))
            .unwrap()
            .unwrap();
        assert_eq!(row.mission_id.as_deref(), Some(task.as_str()));

        set_task_status(&db, &task, t::STATUS_BLOCKED);
        session(&db, "hs-old", None);
        db.with_conn(|c| registry::set_status(c, "hs-old", registry::STATUS_EXITED))
            .unwrap();
        let linked = db.with_conn(|c| link(c, "hs-old", &task)).unwrap();
        assert!(!linked.moved);
        assert_eq!(status(&db, &task), t::STATUS_BLOCKED);

        assert!(db.with_conn(|c| link(c, "hs-missing", &task)).is_err());
        assert!(db.with_conn(|c| link(c, "hs-live", "FR-999")).is_err());
    }

    #[test]
    fn a_session_without_a_task_or_with_a_stale_mission_id_records_nothing() {
        let (db, task) = setup();
        let none = session(&db, "hs-none", None);
        let stale = session(&db, "hs-stale", Some("m-retired-engine"));
        assert_eq!(apply(&db, &none, Event::Launched), None);
        assert_eq!(apply(&db, &stale, Event::Exited), None);
        assert!(comments(&db, &task).is_empty());
    }
}
