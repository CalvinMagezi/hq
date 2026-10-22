//! The pipeline end to end, on an in-memory database, canned GitHub answers and a scripted
//! reviewer: no network and no model.

use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use hq_core::config::{VerificationConfig, VerificationMode};
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};

use super::pipeline::*;
use super::remote::GitHubRead;
use super::remote::fake::FakeGitHub;
use super::reviewer::fake::{ScriptedReviewer, reply};

const CLEAN_DIFF: &str = "diff --git a/src/guard.rs b/src/guard.rs\n--- a/src/guard.rs\n+++ b/src/guard.rs\n@@ -1 +1,3 @@\n+fn reject_closed_jobs() {}\n";
const SKIP_DIFF: &str = "diff --git a/src/guard.rs b/src/guard.rs\n--- a/src/guard.rs\n+++ b/src/guard.rs\n@@ -1 +1,2 @@\n+fn reject_closed_jobs() {}\ndiff --git a/src/guard_test.go b/src/guard_test.go\n--- a/src/guard_test.go\n+++ b/src/guard_test.go\n@@ -1 +1,2 @@\n+\tt.Skip(\"flaky\")\n";
const DESCRIPTION: &str = "Reject applications to closed jobs.\n\nAcceptance:\n- closed jobs refuse applications\n- tests cover it\n";

fn enforce() -> VerificationConfig {
    VerificationConfig {
        mode: VerificationMode::Enforce,
        reviewer_monthly_ceiling_usd: Some(50.0),
        ..Default::default()
    }
}

fn db_with_task(description: &str) -> Database {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'W')",
            [],
        )?;
        t::create_task(
            c,
            "tk-1",
            "in-1",
            &t::NewTask {
                title: "Reject closed jobs",
                description,
                created_by: "test",
                ..Default::default()
            },
        )?;
        Ok(())
    })
    .unwrap();
    sync(&db, &enforce());
    db
}

fn sync(db: &Database, cfg: &VerificationConfig) {
    db.with_conn(|c| t::sync_verification_policy(c, cfg))
        .unwrap();
}

fn set_status(db: &Database, status: &str, actor: &str) {
    let ctx = t::WriteCtx {
        actor: Some(actor),
        ..Default::default()
    };
    db.with_conn(|c| {
        t::update_task_as(
            c,
            "tk-1",
            &t::TaskPatch {
                status: Some(status.into()),
                ..Default::default()
            },
            None,
            &ctx,
        )
    })
    .unwrap();
}

/// A task a worker has done and submitted.
fn submitted(db: &Database) {
    set_status(db, t::STATUS_IN_PROGRESS, "worker");
    set_status(db, t::STATUS_READY_FOR_REVIEW, "worker");
}

fn link(db: &Database, kind: &str, reference: &str, direction: &str) {
    db.with_conn(|c| t::add_task_link(c, "tk-1", kind, reference, "", Some(direction), "t"))
        .unwrap();
}

fn get(db: &Database) -> t::Task {
    db.with_conn(|c| Ok(t::get_task(c, "tk-1")?.unwrap()))
        .unwrap()
}

fn verdicts(db: &Database) -> Vec<t::Verdict> {
    db.with_conn(|c| t::list_verdicts(c, "tk-1", 50)).unwrap()
}

fn merged_pr_github(diff: &str) -> FakeGitHub {
    FakeGitHub::default()
        .with(
            "repos/example/repo/pulls/7",
            json!({"state": "closed", "merged": true, "head": {"sha": "abc123"}, "base": {"ref": "main"},
                   "user": {"login": "dev"}, "changed_files": 1, "mergeable": null}),
        )
        .with(
            "repos/example/repo/commits/abc123/check-runs?per_page=100",
            json!({"check_runs": [{"name": "test", "status": "completed", "conclusion": "success"}]}),
        )
        .with(
            "repos/example/repo/branches/main/protection/required_status_checks",
            json!({"contexts": ["test"]}),
        )
        .with_diff("example/repo", 7, diff, false)
}

struct World {
    db: Database,
    gh: Arc<dyn GitHubRead>,
    vault: tempfile::TempDir,
    cfg: VerificationConfig,
}

impl World {
    fn code(gh: FakeGitHub) -> Self {
        let db = db_with_task(DESCRIPTION);
        link(&db, "pr", "example/repo#7", "produced");
        submitted(&db);
        Self {
            db,
            gh: Arc::new(gh),
            vault: tempfile::tempdir().unwrap(),
            cfg: enforce(),
        }
    }

    fn env(&self) -> Env<'_> {
        Env {
            db: &self.db,
            gh: self.gh.as_ref(),
            vault_path: self.vault.path(),
            cfg: &self.cfg,
            route_tags: true,
        }
    }

    async fn evidence(&self) -> Evidence {
        gather(&self.env(), &get(&self.db)).await.unwrap()
    }

    async fn verify(&self, reviewer: &ScriptedReviewer) -> Finished {
        verify_task(&self.env(), reviewer, &get(&self.db), true)
            .await
            .unwrap()
    }

    /// A reviewer's JSON answering every criterion as `result`, citing the given observation kinds.
    async fn answer(&self, result: &str, kinds: &[&str], flags: &[(&str, bool)]) -> String {
        let ev = self.evidence().await;
        let ids: Vec<String> = kinds
            .iter()
            .filter_map(|k| ev.log.of_kind(k).next().map(|o| o.id.clone()))
            .collect();
        answer_json(&ev.contract.criteria, result, &ids, flags)
    }
}

fn answer_json(
    criteria: &[String],
    result: &str,
    ids: &[String],
    flags: &[(&str, bool)],
) -> String {
    json!({
        "criteria": criteria.iter().map(|c| json!({"text": c, "result": result, "observation_ids": ids, "note": format!("{result}: {c}")})).collect::<Vec<_>>(),
        "flags_addressed": flags.iter().map(|(k, j)| json!({"kind": k, "justified": j, "note": "because"})).collect::<Vec<_>>(),
        "summary": "Looks done.",
    })
    .to_string()
}

fn completed(f: &Finished) -> bool {
    matches!(
        f,
        Finished::Recorded {
            moved: Moved::Completed,
            ..
        }
    )
}

#[test]
fn the_ledger_origin_the_database_reads_is_the_one_the_reviewer_writes() {
    assert_eq!(hq_llm::origin::VERIFICATION, "verification");
}

#[tokio::test]
async fn a_merged_pr_with_green_required_checks_and_a_cited_review_completes_the_task() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w
        .answer(
            "pass",
            &["pr_state", "required_checks", "diff_excerpt"],
            &[],
        )
        .await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("anthropic/claude-x", &answer)]);

    let done = w.verify(&reviewer).await;
    assert!(completed(&done), "{done:?}");
    assert_eq!(get(&w.db).status, t::STATUS_COMPLETE);
    let v = &verdicts(&w.db)[0];
    assert_eq!(
        (v.verdict.as_str(), v.level.as_str(), v.tier),
        ("pass", "machine_peer", 1)
    );
    assert!(v.applied && v.verifier.starts_with("verify:"));
    assert_eq!(v.artifact, "pr:example/repo#7");
    assert!(v.criteria[0]["observation_ids"].as_array().unwrap().len() >= 2);
    // The completion is attributed to the verifier and says how it was verified, in quoted words.
    let events = w.db.with_conn(|c| t::list_task_events(c, "tk-1")).unwrap();
    assert_eq!(
        events.last().unwrap().actor.as_deref(),
        Some(v.verifier.as_str())
    );
    let comments = w.db.with_conn(|c| t::list_comments(c, "tk-1")).unwrap();
    let note = &comments.last().unwrap().body;
    assert!(
        note.starts_with("Verified complete by HQ: machine_peer evidence, tier 1"),
        "{note}"
    );
    assert!(note.contains("> Looks done."), "{note}");
    assert_eq!(reviewer.calls(), 1);
}

#[tokio::test]
async fn shadow_mode_records_the_verdict_and_moves_nothing() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.mode = VerificationMode::Shadow;
    sync(&w.db, &w.cfg);
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
    let done = w.verify(&reviewer).await;
    assert!(!completed(&done), "{done:?}");
    assert_eq!(
        get(&w.db).status,
        t::STATUS_READY_FOR_REVIEW,
        "nothing moved"
    );
    let v = &verdicts(&w.db)[0];
    assert_eq!(v.verdict, "pass");
    assert!(!v.applied, "recorded as unapplied");
}

#[tokio::test]
async fn a_tier_that_is_not_enforced_yet_behaves_like_shadow() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.enforce_tiers = vec![0];
    sync(&w.db, &w.cfg);
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
    let done = w.verify(&reviewer).await;
    assert!(!completed(&done));
    assert_eq!(
        get(&w.db).status,
        t::STATUS_READY_FOR_REVIEW,
        "tier 1 is not staged in yet"
    );
}

#[tokio::test]
async fn with_no_reviewer_ceiling_no_model_is_called_and_nothing_is_decided_by_guess() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.reviewer_monthly_ceiling_usd = None;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(&done, Finished::Deferred { waiting_for: WaitingFor::Budget, reason, .. } if reason.contains("reviewer_monthly_ceiling_usd")),
        "{done:?}"
    );
    assert_eq!(reviewer.calls(), 0, "no model was called");
    assert!(verdicts(&w.db).is_empty(), "a deferral records no verdict");
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

#[tokio::test]
async fn deterministic_failures_land_without_a_reviewer_configured() {
    let gh = FakeGitHub::default()
        .with(
            "repos/example/repo/pulls/7",
            json!({"state": "open", "merged": false, "head": {"sha": "abc123"}, "base": {"ref": "main"}, "mergeable": true}),
        )
        .with(
            "repos/example/repo/commits/abc123/check-runs?per_page=100",
            json!({"check_runs": [{"name": "test", "status": "completed", "conclusion": "failure"}]}),
        )
        .with_diff("example/repo", 7, CLEAN_DIFF, false);
    let mut w = World::code(gh);
    w.cfg.reviewer_monthly_ceiling_usd = None;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let done = w.verify(&reviewer).await;
    let Finished::Recorded { verdict, moved, .. } = done else {
        panic!("expected a recorded verdict");
    };
    assert_eq!(verdict.verdict, "fail");
    assert!(verdict.reasons.contains("failed"), "{}", verdict.reasons);
    assert_eq!(
        moved,
        Moved::Returned,
        "a failed task goes back to its worker"
    );
    assert_eq!(get(&w.db).status, t::STATUS_TO_DO);
    assert_eq!(reviewer.calls(), 0);
}

#[tokio::test]
async fn a_review_that_cites_nothing_or_an_unrecorded_fact_does_not_pass() {
    for ids in [json!([]), json!(["o999"])] {
        let w = World::code(merged_pr_github(CLEAN_DIFF));
        let ev = w.evidence().await;
        let answer = json!({"criteria": ev.contract.criteria.iter()
            .map(|c| json!({"text": c, "result": "pass", "observation_ids": ids, "note": "fine"})).collect::<Vec<_>>(),
            "summary": "done"})
        .to_string();
        let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
        let done = w.verify(&reviewer).await;
        let Finished::Recorded { verdict, moved, .. } = done else {
            panic!("expected a recorded verdict, got {done:?}");
        };
        assert_eq!(verdict.verdict, "inconclusive", "{ids}");
        assert!(
            verdict.reasons.contains("without citing"),
            "{}",
            verdict.reasons
        );
        // Not shown, so not passed: back to the worker to make it demonstrable.
        assert_eq!(moved, Moved::Returned);
        assert_eq!(get(&w.db).status, t::STATUS_TO_DO);
    }
}

#[tokio::test]
async fn a_failed_criterion_fails_the_task_with_the_reviewers_reason() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w.answer("fail", &["pr_state"], &[]).await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
    let Finished::Recorded { verdict, moved, .. } = w.verify(&reviewer).await else {
        panic!("expected a verdict");
    };
    assert!(moved != Moved::Completed);
    assert_eq!(verdict.verdict, "fail");
    assert!(
        verdict.reasons.contains("closed jobs refuse applications"),
        "{}",
        verdict.reasons
    );
}

#[tokio::test]
async fn an_unusable_answer_defers_to_the_verifier_and_records_nothing() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", "looks fine to me")]);
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(
            done,
            Finished::Deferred {
                waiting_for: WaitingFor::Verifier,
                ..
            }
        ),
        "{done:?}"
    );
    assert!(verdicts(&w.db).is_empty());

    let reviewer = ScriptedReviewer::new(&["premium"], vec![Err(anyhow!("timed out"))]);
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(
            done,
            Finished::Deferred {
                waiting_for: WaitingFor::Verifier,
                ..
            }
        ),
        "{done:?}"
    );
}

#[tokio::test]
async fn a_new_head_commit_during_review_voids_the_verdict_before_it_is_applied() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    // After the review, GitHub reports another head commit.
    let swapped = Arc::new(Mutex::new(false));
    struct Flip(FakeGitHub, Arc<Mutex<bool>>);
    #[async_trait::async_trait]
    impl GitHubRead for Flip {
        async fn api_json(&self, path: &str) -> anyhow::Result<Value> {
            let mut v = self.0.api_json(path).await?;
            if *self.1.lock().unwrap() && path == "repos/example/repo/pulls/7" {
                v["head"]["sha"] = json!("def456");
            }
            Ok(v)
        }
        async fn pr_diff(&self, repo: &str, n: u64) -> anyhow::Result<super::remote::Diff> {
            self.0.pr_diff(repo, n).await
        }
    }
    let mut w = w;
    w.gh = Arc::new(Flip(merged_pr_github(CLEAN_DIFF), swapped.clone()));
    let flag = swapped.clone();
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)])
        .on_review(move || *flag.lock().unwrap() = true);
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(&done, Finished::Voided { reason } if reason.contains("new head commit")),
        "{done:?}"
    );
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
    assert!(
        verdicts(&w.db).is_empty(),
        "a voided pass is not recorded as one"
    );
}

#[tokio::test]
async fn a_note_edited_during_review_voids_the_verdict() {
    let db = db_with_task("Write the summary.\n\nAcceptance:\n- the summary states the decision\n");
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    std::fs::write(
        vault.path().join("Notebooks/summary.md"),
        "We decided to ship.",
    )
    .unwrap();
    link(&db, "vault_note", "Notebooks/summary.md", "produced");
    submitted(&db);
    let w = World {
        db,
        gh: Arc::new(FakeGitHub::default()),
        vault,
        cfg: enforce(),
    };
    let ev = w.evidence().await;
    let id = ev.log.of_kind("note_excerpt").next().unwrap().id.clone();
    let answer = answer_json(&ev.contract.criteria, "pass", &[id], &[]);
    let path = w.vault.path().join("Notebooks/summary.md");
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)])
        .on_review(move || std::fs::write(&path, "We decided NOT to ship.").unwrap());
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(&done, Finished::Voided { reason } if reason.contains("edited since")),
        "{done:?}"
    );
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

#[tokio::test]
async fn a_note_task_is_reviewed_at_tier_zero_for_peer_level_evidence() {
    let db = db_with_task("Write the summary.\n\nAcceptance:\n- the summary states the decision\n");
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    std::fs::write(
        vault.path().join("Notebooks/summary.md"),
        "We decided to ship.",
    )
    .unwrap();
    link(&db, "vault_note", "Notebooks/summary.md", "produced");
    submitted(&db);
    let w = World {
        db,
        gh: Arc::new(FakeGitHub::default()),
        vault,
        cfg: enforce(),
    };
    let ev = w.evidence().await;
    assert_eq!(ev.tier, 0);
    let id = ev.log.of_kind("note_excerpt").next().unwrap().id.clone();
    let answer = answer_json(&ev.contract.criteria, "pass", &[id], &[]);
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
    let done = w.verify(&reviewer).await;
    assert!(completed(&done), "{done:?}");
    assert_eq!(verdicts(&w.db)[0].level, "peer");
    assert_eq!(get(&w.db).status, t::STATUS_COMPLETE);
}

#[tokio::test]
async fn work_changed_while_it_was_being_verified_is_not_completed_by_a_stale_verdict() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    let db = w.db.clone();
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]).on_review(
        move || {
            // The worker takes it back and submits it again.
            let ctx = t::WriteCtx {
                actor: Some("worker"),
                ..Default::default()
            };
            for s in [t::STATUS_IN_PROGRESS, t::STATUS_READY_FOR_REVIEW] {
                db.with_conn(|c| {
                    t::update_task_as(
                        c,
                        "tk-1",
                        &t::TaskPatch {
                            status: Some(s.into()),
                            ..Default::default()
                        },
                        None,
                        &ctx,
                    )
                })
                .unwrap();
            }
        },
    );
    let done = w.verify(&reviewer).await;
    assert!(
        matches!(&done, Finished::Voided { reason } if reason.contains("submitted again")),
        "{done:?}"
    );
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

#[tokio::test]
async fn nothing_to_inspect_is_inconclusive_and_sent_back_to_the_worker() {
    let db = db_with_task(DESCRIPTION);
    submitted(&db);
    let w = World {
        db,
        gh: Arc::new(FakeGitHub::default()),
        vault: tempfile::tempdir().unwrap(),
        cfg: enforce(),
    };
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let Finished::Recorded {
        verdict,
        waiting_for,
        moved,
    } = w.verify(&reviewer).await
    else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "inconclusive");
    assert_eq!(waiting_for, Some(WaitingFor::Worker));
    assert!(
        verdict.reasons.contains("nothing to inspect"),
        "{}",
        verdict.reasons
    );
    // Never completed, and sent back with what to link.
    assert_eq!(moved, Moved::Returned);
    assert_eq!(get(&w.db).status, t::STATUS_TO_DO);
    assert_eq!(verdict.artifact, "none");
    assert_eq!(reviewer.calls(), 0);
}

#[tokio::test]
async fn a_task_with_open_sub_tasks_fails_before_anyone_reads_a_line() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    w.db.with_conn(|c| {
        t::create_task(
            c,
            "tk-2",
            "in-1",
            &t::NewTask {
                title: "part two",
                parent_task_id: Some("tk-1"),
                created_by: "test",
                ..Default::default()
            },
        )
        .map(|_| ())
    })
    .unwrap();
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let Finished::Recorded { verdict, .. } = w.verify(&reviewer).await else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "fail");
    assert!(
        verdict.reasons.contains("sub-tasks are not complete"),
        "{}",
        verdict.reasons
    );
    assert_eq!(reviewer.calls(), 0);
}

#[tokio::test]
async fn code_with_no_stated_criteria_is_sent_back_not_assessed() {
    let db = db_with_task("Just do the thing, no criteria written.");
    link(&db, "pr", "example/repo#7", "produced");
    submitted(&db);
    let w = World {
        db,
        gh: Arc::new(merged_pr_github(CLEAN_DIFF)),
        vault: tempfile::tempdir().unwrap(),
        cfg: enforce(),
    };
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let Finished::Recorded {
        verdict,
        waiting_for,
        ..
    } = w.verify(&reviewer).await
    else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "inconclusive");
    assert_eq!(waiting_for, Some(WaitingFor::Worker));
    assert!(
        verdict.reasons.contains("Acceptance section"),
        "{}",
        verdict.reasons
    );
}

#[tokio::test]
async fn checks_still_running_or_a_required_check_not_yet_reported_wait_for_ci() {
    for runs in [
        json!([{"name": "test", "status": "in_progress", "conclusion": null}]),
        json!([{"name": "lint", "status": "completed", "conclusion": "success"}]),
    ] {
        let gh = merged_pr_github(CLEAN_DIFF).with(
            "repos/example/repo/commits/abc123/check-runs?per_page=100",
            json!({ "check_runs": runs }),
        );
        let w = World::code(gh);
        let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
        let done = w.verify(&reviewer).await;
        assert!(
            matches!(
                done,
                Finished::Deferred {
                    waiting_for: WaitingFor::Ci,
                    ..
                }
            ),
            "{done:?}"
        );
        assert!(
            verdicts(&w.db).is_empty(),
            "waiting for CI is not a verdict"
        );
        assert_eq!(reviewer.calls(), 0);
    }
}

#[tokio::test]
async fn an_open_pr_with_conflicts_or_a_draft_is_sent_back_and_a_clean_open_one_can_pass() {
    let open = |mergeable: Value, draft: bool| {
        merged_pr_github(CLEAN_DIFF).with(
            "repos/example/repo/pulls/7",
            json!({"state": "open", "merged": false, "draft": draft, "head": {"sha": "abc123"},
                   "base": {"ref": "main"}, "mergeable": mergeable}),
        )
    };
    for (mergeable, draft, why) in [
        (json!(false), false, "conflicts"),
        (json!(true), true, "draft"),
    ] {
        let w = World::code(open(mergeable, draft));
        let Finished::Recorded { verdict, .. } =
            w.verify(&ScriptedReviewer::new(&["premium"], vec![])).await
        else {
            panic!("expected a verdict");
        };
        assert_eq!(verdict.verdict, "inconclusive");
        assert!(verdict.reasons.contains(why), "{}", verdict.reasons);
    }
    let w = World::code(open(json!(true), false));
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    let done = w
        .verify(&ScriptedReviewer::new(
            &["premium"],
            vec![reply("claude-x", &answer)],
        ))
        .await;
    assert!(
        completed(&done),
        "an open, mergeable PR with green checks is landable: {done:?}"
    );
}

#[tokio::test]
async fn code_with_no_checks_needs_the_peer_only_switch() {
    // A repository with no CI at all: nothing reported, and no protection requiring any check.
    let no_checks = || {
        FakeGitHub::default()
            .with(
                "repos/example/repo/pulls/7",
                json!({"state": "closed", "merged": true, "head": {"sha": "abc123"}, "base": {"ref": "main"}}),
            )
            .with(
                "repos/example/repo/commits/abc123/check-runs?per_page=100",
                json!({ "check_runs": [] }),
            )
            .with_diff("example/repo", 7, CLEAN_DIFF, false)
    };
    let w = World::code(no_checks());
    let Finished::Recorded { verdict, .. } =
        w.verify(&ScriptedReviewer::new(&["premium"], vec![])).await
    else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "inconclusive");
    assert!(
        verdict.reasons.contains("peer_only_code"),
        "{}",
        verdict.reasons
    );

    let mut w = World::code(no_checks());
    w.cfg.peer_only_code = true;
    let answer = w.answer("pass", &["pr_state", "diff_excerpt"], &[]).await;
    let done = w
        .verify(&ScriptedReviewer::new(
            &["premium"],
            vec![reply("claude-x", &answer)],
        ))
        .await;
    assert!(completed(&done), "{done:?}");
    assert_eq!(
        verdicts(&w.db)[0].level,
        "peer",
        "recorded as peer evidence, not machine"
    );
}

#[tokio::test]
async fn a_flag_on_the_change_raises_the_tier_and_needs_two_model_families() {
    let w = World::code(merged_pr_github(SKIP_DIFF));
    let ev = w.evidence().await;
    assert_eq!(ev.tier, 2, "a skipped test raises a tier 1 task to tier 2");
    assert!(ev.flags.contains(&"skipped_test".to_string()));

    // One family answering is not enough.
    let one = w
        .answer(
            "pass",
            &["pr_state", "required_checks"],
            &[("skipped_test", true)],
        )
        .await;
    let single = ScriptedReviewer::new(
        &["a", "b"],
        vec![reply("claude-x", &one), reply("claude-y", &one)],
    );
    let done = w.verify(&single).await;
    assert!(
        matches!(&done, Finished::Deferred { waiting_for: WaitingFor::Owner, reason, .. } if reason.contains("model families")),
        "two answers from one family are one opinion: {done:?}"
    );

    // Two families, both justifying the flag: it passes, and says the evidence was independent.
    let two = ScriptedReviewer::new(
        &["a", "b"],
        vec![reply("claude-x", &one), reply("openai/gpt-5", &one)],
    );
    let done = w.verify(&two).await;
    assert!(completed(&done), "{done:?}");
    let v = &verdicts(&w.db)[0];
    assert_eq!(
        (v.tier, v.independence.as_str(), v.level.as_str()),
        (2, "different_model", "machine_peer")
    );
    assert_eq!(two.calls(), 2);
}

#[tokio::test]
async fn a_flag_the_reviewer_does_not_justify_fails_and_one_it_ignores_is_inconclusive() {
    let w = World::code(merged_pr_github(SKIP_DIFF));
    let unjustified = w
        .answer(
            "pass",
            &["pr_state", "required_checks"],
            &[("skipped_test", false)],
        )
        .await;
    let reviewers = ScriptedReviewer::new(
        &["a", "b"],
        vec![
            reply("claude-x", &unjustified),
            reply("openai/gpt-5", &unjustified),
        ],
    );
    let Finished::Recorded { verdict, moved, .. } = w.verify(&reviewers).await else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "fail");
    assert!(
        moved != Moved::Completed && verdict.reasons.contains("weakens a check without a reason"),
        "{}",
        verdict.reasons
    );

    let w = World::code(merged_pr_github(SKIP_DIFF));
    let ignored = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    let reviewers = ScriptedReviewer::new(
        &["a", "b"],
        vec![reply("claude-x", &ignored), reply("openai/gpt-5", &ignored)],
    );
    let Finished::Recorded { verdict, .. } = w.verify(&reviewers).await else {
        panic!("expected a verdict");
    };
    assert_eq!(verdict.verdict, "inconclusive");
    assert!(
        verdict
            .reasons
            .contains("did not address these flags: skipped_test"),
        "{}",
        verdict.reasons
    );
}

#[tokio::test]
async fn the_reviewer_is_shown_only_recorded_facts_and_the_task_as_data() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w.answer("unclear", &["pr_state"], &[]).await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("claude-x", &answer)]);
    w.verify(&reviewer).await;
    let asked = reviewer.asked.lock().unwrap();
    let request = &asked[0];
    assert_eq!(
        request.criteria,
        ["closed jobs refuse applications", "tests cover it"]
    );
    let ids: Vec<&str> = request
        .observations
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    assert!(ids.contains(&"o1") && ids.len() >= 4, "{ids:?}");
    assert!(
        request
            .observations
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "diff_excerpt")
    );
    assert_eq!(request.task.tier, 1);
}

#[tokio::test]
async fn a_completed_or_already_verified_task_is_not_in_the_queue() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let cfg = w.cfg.clone();
    let queued = |db: &Database| {
        db.with_conn(|c| t::verification_candidates(c, &cfg, chrono::Utc::now().timestamp()))
            .unwrap()
            .len()
    };
    assert_eq!(queued(&w.db), 1);
    let answer = w
        .answer("pass", &["pr_state", "required_checks"], &[])
        .await;
    w.verify(&ScriptedReviewer::new(
        &["premium"],
        vec![reply("claude-x", &answer)],
    ))
    .await;
    assert_eq!(queued(&w.db), 0, "verified and complete");
}

// ---- dispatch ----

use super::dispatch::dispatch;

async fn run_pass(w: &World, reviewer: &ScriptedReviewer) -> super::dispatch::DispatchReport {
    dispatch(&w.env(), reviewer).await.unwrap()
}

#[tokio::test]
async fn dispatch_does_nothing_while_verification_is_off() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.mode = VerificationMode::Off;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let report = run_pass(&w, &reviewer).await;
    assert_eq!(report, Default::default());
    assert!(verdicts(&w.db).is_empty());
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

#[tokio::test]
async fn a_pass_completes_the_waiting_tasks_and_leaves_an_empty_queue() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let answer = w
        .answer(
            "pass",
            &["pr_state", "required_checks", "diff_excerpt"],
            &[],
        )
        .await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("anthropic/claude-x", &answer)]);

    let report = run_pass(&w, &reviewer).await;
    assert_eq!(
        (
            report.queued,
            report.attempted,
            report.recorded,
            report.completed
        ),
        (1, 1, 1, 1),
        "{report:?}"
    );
    assert_eq!(get(&w.db).status, t::STATUS_COMPLETE);

    let again = run_pass(&w, &reviewer).await;
    assert_eq!((again.queued, again.attempted), (0, 0));
    assert_eq!(reviewer.calls(), 1, "a verified task is not reviewed again");
}

#[tokio::test]
async fn a_deferred_task_waits_out_its_retry_but_a_resubmission_is_looked_at_at_once() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    // No ceiling: the task needs a reviewer, which is deferred, not failed.
    w.cfg.reviewer_monthly_ceiling_usd = None;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);

    let first = run_pass(&w, &reviewer).await;
    assert_eq!(
        (first.attempted, first.deferred, first.recorded),
        (1, 1, 0),
        "{first:?}"
    );
    assert!(verdicts(&w.db).is_empty(), "a deferral records nothing");
    let wait =
        w.db.with_conn(|c| t::current_wait(c, "tk-1"))
            .unwrap()
            .unwrap();
    assert_eq!((wait.waiting_for.as_str(), wait.tries), ("budget", 1));
    assert!(wait.reason.contains("ceiling"), "{}", wait.reason);

    let second = run_pass(&w, &reviewer).await;
    assert_eq!((second.queued, second.attempted), (0, 0), "{second:?}");

    // The worker sends it back and submits again: a new submission is not in any wait.
    set_status(&w.db, t::STATUS_IN_PROGRESS, "worker");
    set_status(&w.db, t::STATUS_READY_FOR_REVIEW, "worker");
    let third = run_pass(&w, &reviewer).await;
    assert_eq!((third.queued, third.attempted), (1, 1), "{third:?}");
    assert_eq!(reviewer.calls(), 0);
    let wait =
        w.db.with_conn(|c| t::current_wait(c, "tk-1"))
            .unwrap()
            .unwrap();
    assert_eq!(wait.tries, 1, "a new submission counts from one");
}

#[tokio::test]
async fn shadow_mode_records_what_it_would_decide_and_moves_nothing() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.mode = VerificationMode::Shadow;
    w.cfg.shadow_e2_sample = 1;
    sync(&w.db, &w.cfg);
    let answer = w
        .answer(
            "pass",
            &["pr_state", "required_checks", "diff_excerpt"],
            &[],
        )
        .await;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("anthropic/claude-x", &answer)]);

    let report = run_pass(&w, &reviewer).await;
    assert_eq!((report.recorded, report.completed), (1, 0), "{report:?}");
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
    let v = &verdicts(&w.db)[0];
    assert_eq!((v.verdict.as_str(), v.applied), ("pass", false));
    assert_eq!(
        w.db.with_conn(t::reviewer_runs_recorded).unwrap(),
        1,
        "the sample counts reviewer runs"
    );
}

#[tokio::test]
async fn shadow_mode_stops_calling_the_reviewer_once_its_sample_is_used() {
    let mut w = World::code(merged_pr_github(CLEAN_DIFF));
    w.cfg.mode = VerificationMode::Shadow;
    w.cfg.shadow_e2_sample = 0;
    sync(&w.db, &w.cfg);
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);

    let report = run_pass(&w, &reviewer).await;
    assert_eq!(report.recorded, 1, "{report:?}");
    assert_eq!(reviewer.calls(), 0);
    let v = &verdicts(&w.db)[0];
    assert_eq!(v.verdict, "inconclusive");
    assert!(v.reasons.contains("sample is used up"), "{}", v.reasons);
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

#[tokio::test]
async fn a_verdict_about_work_that_was_submitted_again_during_the_check_is_dropped() {
    let w = World::code(merged_pr_github(CLEAN_DIFF));
    let ev = w.evidence().await;
    // The worker resubmits after the facts were gathered and before the verdict is recorded.
    set_status(&w.db, t::STATUS_IN_PROGRESS, "worker");
    set_status(&w.db, t::STATUS_READY_FOR_REVIEW, "worker");
    let recorded = finish(
        &w.env(),
        &ev,
        Resolution::Fail {
            reasons: "tests do not pass".into(),
            criteria: json!([]),
        },
    )
    .await
    .unwrap();
    assert!(matches!(recorded, Finished::Voided { .. }), "{recorded:?}");
    assert!(verdicts(&w.db).is_empty());
}

fn failing_checks_github() -> FakeGitHub {
    FakeGitHub::default()
        .with(
            "repos/example/repo/pulls/7",
            json!({"state": "closed", "merged": true, "head": {"sha": "abc123"}, "base": {"ref": "main"},
                   "user": {"login": "dev"}, "changed_files": 1, "mergeable": null}),
        )
        .with(
            "repos/example/repo/commits/abc123/check-runs?per_page=100",
            json!({"check_runs": [{"name": "test", "status": "completed", "conclusion": "failure"}]}),
        )
        .with_diff("example/repo", 7, CLEAN_DIFF, false)
}

#[tokio::test]
async fn a_failing_task_is_sent_back_with_findings_and_the_third_failure_goes_to_the_owner() {
    let w = World::code(failing_checks_github());
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let mut seen = Vec::new();
    for round in 0..3 {
        if round > 0 {
            set_status(&w.db, t::STATUS_IN_PROGRESS, "worker");
            set_status(&w.db, t::STATUS_READY_FOR_REVIEW, "worker");
        }
        let report = run_pass(&w, &reviewer).await;
        seen.push((report.returned, report.escalated, get(&w.db).status));
    }
    assert_eq!(
        seen,
        [
            (1, 0, t::STATUS_TO_DO.to_string()),
            (1, 0, t::STATUS_TO_DO.to_string()),
            (0, 1, t::STATUS_BLOCKED.to_string()),
        ]
    );
    let task = get(&w.db);
    assert_eq!(task.waiting_on.as_deref(), Some("owner"));
    let checkpoint =
        w.db.with_conn(|c| t::latest_checkpoint(c, "tk-1"))
            .unwrap()
            .unwrap();
    assert!(
        checkpoint.summary.contains("failed"),
        "{}",
        checkpoint.summary
    );
    assert_eq!(verdicts(&w.db).len(), 3);
    assert!(
        verdicts(&w.db)
            .iter()
            .all(|v| v.waiting_for.as_deref() == Some("worker"))
    );
}

#[tokio::test]
async fn nothing_is_sent_back_while_verification_only_watches() {
    let mut w = World::code(failing_checks_github());
    w.cfg.mode = VerificationMode::Shadow;
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let report = run_pass(&w, &reviewer).await;
    assert_eq!((report.recorded, report.returned), (1, 0), "{report:?}");
    assert_eq!(get(&w.db).status, t::STATUS_READY_FOR_REVIEW);
}

// ---- the whole loop, through the tools an agent uses ----

fn tools_for(w: &World) -> Vec<Box<dyn crate::registry::HqTool>> {
    let settings = hq_core::config::TasksConfig {
        verification: w.cfg.clone(),
        ..Default::default()
    };
    crate::tasks::build_task_tools(
        w.vault.path().to_path_buf(),
        Arc::new(hq_vault::VaultClient::new(w.vault.path().to_path_buf()).unwrap()),
        Arc::new(w.db.clone()),
        settings,
        None,
        Arc::new(super::remote::fake::FakeGitHub::default()),
    )
}

async fn tool(
    tools: &[Box<dyn crate::registry::HqTool>],
    name: &str,
    args: Value,
) -> anyhow::Result<Value> {
    tools
        .iter()
        .find(|t| t.name() == name)
        .unwrap()
        .execute(args)
        .await
}

#[tokio::test]
async fn work_is_sent_back_once_then_passes_and_the_owner_is_never_asked() {
    // The first submission, made by the worker, fails on a red check.
    let w = World::code(failing_checks_github());
    w.db.with_conn(|c| {
        t::update_task(
            c,
            "tk-1",
            &t::TaskPatch {
                assignees: Some(vec!["worker".into()]),
                ..Default::default()
            },
            None,
        )
    })
    .unwrap();
    let reviewer = ScriptedReviewer::new(&["premium"], vec![]);
    let first = run_pass(&w, &reviewer).await;
    assert_eq!((first.returned, first.completed), (1, 0), "{first:?}");

    // Anyone looking at the task is told who acts next.
    let tools = tools_for(&w);
    let sent_back = tool(&tools, "task_get", json!({"id": "tk-1"}))
        .await
        .unwrap();
    assert_eq!(sent_back["waiting_for"]["who"], "worker", "{sent_back}");
    assert!(
        sent_back["waiting_for"]["reason"]
            .as_str()
            .unwrap()
            .contains("failed")
    );

    // The worker's next task_next hands it back with what to fix.
    let next = tool(&tools, "task_next", json!({"actor": "worker"}))
        .await
        .unwrap();
    assert_eq!(next["task"]["id"], "tk-1", "{next}");
    let resume = next["resume"].to_string();
    assert!(
        resume.contains("did not pass") && resume.contains("failed"),
        "{resume}"
    );

    // It fixes the problem and hands the work over again, saying complete, as agents do.
    let released = tool(
        &tools,
        "task_release",
        json!({"lease": next["lease"], "status": "complete", "summary": "fixed the failing test"}),
    )
    .await
    .unwrap();
    assert_eq!(released["task"]["status"], "ready_for_review");
    assert_eq!(released["verification_queued"], true);
    let queued = tool(&tools, "task_get", json!({"id": "tk-1"}))
        .await
        .unwrap();
    assert_eq!(queued["waiting_for"]["who"], "verifier", "{queued}");

    // Now the checks are green and a reviewer passes it: HQ completes it, not the worker.
    let green = merged_pr_github(CLEAN_DIFF);
    let env = Env {
        db: &w.db,
        gh: &green,
        vault_path: w.vault.path(),
        cfg: &w.cfg,
        route_tags: true,
    };
    let ev = gather(&env, &get(&w.db)).await.unwrap();
    let ids: Vec<String> = ["pr_state", "required_checks", "diff_excerpt"]
        .iter()
        .filter_map(|k| ev.log.of_kind(k).next().map(|o| o.id.clone()))
        .collect();
    let answer = answer_json(&ev.contract.criteria, "pass", &ids, &[]);
    let reviewer = ScriptedReviewer::new(&["premium"], vec![reply("anthropic/claude-x", &answer)]);
    let second = dispatch(&env, &reviewer).await.unwrap();
    assert_eq!((second.completed, second.returned), (1, 0), "{second:?}");

    let done = get(&w.db);
    assert_eq!(done.status, t::STATUS_COMPLETE);
    let finished = tool(&tools, "task_get", json!({"id": "tk-1"}))
        .await
        .unwrap();
    assert!(finished["waiting_for"].is_null(), "{finished}");
    let all = verdicts(&w.db);
    assert_eq!(all.len(), 2, "one return, one pass");
    assert_eq!(all[0].verdict, "pass");
    assert_eq!(all[1].waiting_for.as_deref(), Some("worker"));
    // Every completion says who made it: a verifier, never the worker.
    let events = w.db.with_conn(|c| t::list_task_events(c, "tk-1")).unwrap();
    let completer = events.last().unwrap().actor.clone().unwrap();
    assert!(completer.starts_with("verify:"), "{completer}");
    assert_eq!(
        w.db.with_conn(|c| t::rejections(c, "tk-1")).unwrap(),
        1,
        "the reopen count survives completion"
    );
}
