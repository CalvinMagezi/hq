//! The check at the moment work is handed over (docs/plans/task-verification.md, "Weekend bursts").
//!
//! The cheapest time to find a pull request is the release, while the agent still has it in
//! context. A code task released with nothing HQ can find to verify gets an immediate reply
//! naming what to link; under `enforce` for an enforced tier the release is refused until it is
//! linked. Most of the unverifiable backlog is created by skipping this step.

use std::time::Duration;

use hq_core::config::{ReleaseCheck, VerificationConfig, VerificationMode};
use hq_db::Database;
use hq_db::tasks as t;

use super::artifact::{TaskFacts, discover, is_shared_branch, repos_for_cwds};
use super::remote::GitHubRead;

/// How long the look-up may take before the release goes ahead with a warning.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// What the check decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// Nothing to say.
    Fine,
    /// The release goes ahead; the reply carries this.
    Warn(String),
    /// The release is refused with this.
    Refuse(String),
}

/// Whether `task`, being handed over for review, has an artifact HQ can find.
pub async fn check_release(
    db: &Database,
    gh: &dyn GitHubRead,
    cfg: &VerificationConfig,
    task: &t::Task,
) -> Checked {
    if cfg.mode == VerificationMode::Off || cfg.release_check == ReleaseCheck::Off {
        return Checked::Fine;
    }
    let loaded = {
        let (cfg, task) = (cfg.clone(), task.clone());
        db.with_conn(move |c| Ok((TaskFacts::load(c, &task.id)?, t::task_tier(c, &task, &cfg)?)))
    };
    let (facts, tier) = match loaded {
        Ok(v) => v,
        // What cannot be read is not a reason to hold a release.
        Err(_) => return Checked::Fine,
    };
    let worked_on_a_branch = facts.branches.iter().any(|b| !is_shared_branch(&b.branch));
    // A vault task needs no pull request: its note, or an assessment, is what is verified.
    if tier == 0 && !worked_on_a_branch {
        return Checked::Fine;
    }
    let cwd_repos = repos_for_cwds(facts.branches.iter().map(|b| b.cwd.as_str()));
    let found = match tokio::time::timeout(LOOKUP_TIMEOUT, discover(gh, &facts, &cwd_repos)).await {
        Ok(found) => found,
        Err(_) => {
            return Checked::Warn(format!(
                "GitHub did not answer in time, so {} could not be checked for a pull request. \
                 Link the pull request (task_link_add, kind pr, reference owner/repo#number) so it can be verified.",
                task.display_id
            ));
        }
    };
    if !found.is_empty() {
        return Checked::Fine;
    }
    let enforced = cfg.mode == VerificationMode::Enforce && cfg.enforces(tier);
    let mut message = format!(
        "HQ cannot find a pull request or commit for {id}, so it cannot be verified{sent_back}. \
         Link it now with task_link_add (task {id}, kind pr, reference owner/repo#number; \
         or kind commit), then release again.",
        id = task.display_id,
        sent_back = if enforced {
            ", and without one it is sent back to you"
        } else {
            ""
        },
    );
    for note in &found.notes {
        message.push(' ');
        message.push_str(note);
        message.push('.');
    }
    // Refuse only when nothing was left unresolved: a failed look-up is not proof of absence.
    let refuse = cfg.release_check == ReleaseCheck::Enforce && enforced && found.notes.is_empty();
    if refuse {
        Checked::Refuse(message)
    } else {
        Checked::Warn(message)
    }
}

#[cfg(test)]
mod tests {
    use super::super::remote::fake::FakeGitHub;
    use super::*;
    use serde_json::json;

    fn db_with_task(tags: &[&str]) -> (Database, t::Task) {
        let db = Database::open_memory().unwrap();
        let task = db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'W')",
                    [],
                )?;
                t::create_task(
                    c,
                    "tk-1",
                    "in-1",
                    &t::NewTask {
                        title: "Ship the guard",
                        created_by: "test",
                        tags: &tags.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        (db, task)
    }

    fn cfg(mode: VerificationMode, check: ReleaseCheck) -> VerificationConfig {
        VerificationConfig {
            mode,
            release_check: check,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn nothing_is_looked_up_while_verification_or_the_check_is_off() {
        let (db, task) = db_with_task(&["code"]);
        let gh = FakeGitHub::default();
        for c in [
            cfg(VerificationMode::Off, ReleaseCheck::Enforce),
            cfg(VerificationMode::Enforce, ReleaseCheck::Off),
        ] {
            assert_eq!(check_release(&db, &gh, &c, &task).await, Checked::Fine);
        }
        assert!(gh.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_vault_task_needs_no_pull_request() {
        let (db, task) = db_with_task(&[]);
        let gh = FakeGitHub::default();
        let c = cfg(VerificationMode::Enforce, ReleaseCheck::Enforce);
        assert_eq!(check_release(&db, &gh, &c, &task).await, Checked::Fine);
    }

    #[tokio::test]
    async fn a_code_task_with_nothing_findable_is_warned_or_refused_by_the_mode() {
        let (db, task) = db_with_task(&["code"]);
        let gh = FakeGitHub::default();
        let warn = check_release(
            &db,
            &gh,
            &cfg(VerificationMode::Enforce, ReleaseCheck::Warn),
            &task,
        )
        .await;
        let Checked::Warn(text) = warn else {
            panic!("expected a warning, got {warn:?}");
        };
        assert!(
            text.contains("task_link_add") && text.contains("owner/repo#number"),
            "{text}"
        );

        let refused = check_release(
            &db,
            &gh,
            &cfg(VerificationMode::Enforce, ReleaseCheck::Enforce),
            &task,
        )
        .await;
        assert!(matches!(refused, Checked::Refuse(_)), "{refused:?}");

        // Shadow mode only watches, so it never refuses.
        let shadow = check_release(
            &db,
            &gh,
            &cfg(VerificationMode::Shadow, ReleaseCheck::Enforce),
            &task,
        )
        .await;
        assert!(matches!(shadow, Checked::Warn(_)), "{shadow:?}");
    }

    #[tokio::test]
    async fn a_linked_pull_request_is_enough() {
        let (db, task) = db_with_task(&["code"]);
        db.with_conn(|c| {
            t::add_task_link(c, "tk-1", "pr", "example/repo#7", "", Some("produced"), "t")
        })
        .unwrap();
        let gh = FakeGitHub::default().with("repos/example/repo/pulls/7", json!({}));
        let c = cfg(VerificationMode::Enforce, ReleaseCheck::Enforce);
        assert_eq!(check_release(&db, &gh, &c, &task).await, Checked::Fine);
    }

    #[tokio::test]
    async fn work_on_a_branch_with_no_known_repository_is_warned_not_refused() {
        let (db, task) = db_with_task(&[]);
        // A lease that worked on a feature branch in a directory HQ cannot map to a repository.
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO task_work_sessions (id, task_id, actor, branch, cwd, token_hash) \
                 VALUES ('ws-1', 'tk-1', 'builder', 'feat/guard', '/nowhere/x', 'h1')",
                [],
            )
            .map(|_| ())
            .map_err(Into::into)
        })
        .unwrap();
        let gh = FakeGitHub::default();
        let c = cfg(VerificationMode::Enforce, ReleaseCheck::Enforce);
        let checked = check_release(&db, &gh, &c, &task).await;
        let Checked::Warn(text) = checked else {
            panic!("expected a warning, got {checked:?}");
        };
        assert!(text.contains("feat/guard"), "{text}");
    }
}
