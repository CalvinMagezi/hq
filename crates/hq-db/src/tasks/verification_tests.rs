use super::tests::{TTL, assign, claim_as, get_one, make, move_to, setup, who};
use super::*;
use crate::Database;
use hq_core::config::{VerificationConfig, VerificationMode};
use serde_json::json;

fn policy(db: &Database, mode: VerificationMode) {
    policy_with(
        db,
        VerificationConfig {
            mode,
            ..Default::default()
        },
    );
}

fn policy_with(db: &Database, cfg: VerificationConfig) {
    db.with_conn(|c| sync_verification_policy(c, &cfg)).unwrap();
}

/// A task that has been through progress and review, ready for a verdict.
fn in_review(db: &Database, initiative: &str, id: &str, worker: &str) {
    make(db, id, initiative, None).unwrap();
    set_status_as(db, id, STATUS_IN_PROGRESS, worker);
    set_status_as(db, id, STATUS_READY_FOR_REVIEW, worker);
}

fn set_status_as(db: &Database, id: &str, status: &str, actor: &str) {
    update_as(
        db,
        id,
        status,
        &WriteCtx {
            actor: Some(actor),
            ..Default::default()
        },
    )
    .unwrap();
}

fn update_as(db: &Database, id: &str, status: &str, ctx: &WriteCtx) -> Result<Task> {
    db.with_conn(|c| {
        update_task_as(
            c,
            id,
            &TaskPatch {
                status: Some(status.into()),
                ..Default::default()
            },
            None,
            ctx,
        )
    })
}

fn pass_for<'a>(
    task: &'a str,
    verifier: &'a str,
    observations: &'a serde_json::Value,
    criteria: &'a serde_json::Value,
) -> NewVerdict<'a> {
    NewVerdict {
        task_id: task,
        artifact: "pr:example/repo#1@abc123",
        contract_hash: "h1",
        tier: 1,
        verdict: VERDICT_PASS,
        level: LEVEL_MACHINE_PEER,
        verifier,
        independence: "different_session",
        observations,
        criteria,
        reasons: "",
        applied: true,
        canary: false,
        submission: None,
        waiting_for: None,
    }
}

fn good_evidence() -> (serde_json::Value, serde_json::Value) {
    (
        json!([{"id": "o1", "source": "github", "fact": "checks passed on abc123"}]),
        json!([{"text": "tests pass", "result": "pass", "observation_ids": ["o1"]}]),
    )
}

fn passing_verdict(db: &Database, task: &str, verifier: &str) -> Verdict {
    let (obs, crit) = good_evidence();
    db.with_conn(|c| record_verdict(c, &pass_for(task, verifier, &obs, &crit)))
        .unwrap()
}

fn needs_verification(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NeedsVerification>().is_some()
}

fn code_task(db: &Database, id: &str) {
    db.with_conn(|c| {
        update_task(
            c,
            id,
            &TaskPatch {
                tags: Some(vec!["code".into()]),
                ..Default::default()
            },
            None,
        )
    })
    .unwrap();
}

#[test]
fn the_policy_is_off_until_a_process_writes_one_and_only_changes_when_it_differs() {
    let (db, _) = setup();
    let cfg = db.with_conn(verification_policy).unwrap();
    assert_eq!(cfg.mode, VerificationMode::Off);
    assert!(!cfg.enforces(0));

    let enforce = VerificationConfig {
        mode: VerificationMode::Enforce,
        ..Default::default()
    };
    assert!(
        db.with_conn(|c| sync_verification_policy(c, &enforce))
            .unwrap()
    );
    assert!(
        !db.with_conn(|c| sync_verification_policy(c, &enforce))
            .unwrap(),
        "the same configuration is not written again"
    );
    assert_eq!(db.with_conn(verification_policy).unwrap(), enforce);
}

#[test]
fn a_tasks_tier_comes_from_its_tags_and_links() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig::default();
    let tier = |db: &Database, id: &str| {
        let task = get_one(db, id);
        db.with_conn(|c| task_tier(c, &task, &cfg)).unwrap()
    };
    make(&db, "plain", &initiative, None).unwrap();
    assert_eq!(tier(&db, "plain"), 0, "vault-only work");

    code_task(&db, "plain");
    assert_eq!(tier(&db, "plain"), 1, "tagged code");

    make(&db, "linked", &initiative, None).unwrap();
    db.with_conn(|c| {
        add_task_link(
            c,
            "linked",
            LINK_PR,
            "example/repo#7",
            "",
            Some("origin"),
            "t",
        )
    })
    .unwrap();
    assert_eq!(tier(&db, "linked"), 1, "linked to a pull request");

    db.with_conn(|c| {
        update_task(
            c,
            "linked",
            &TaskPatch {
                tags: Some(vec!["Production".into()]),
                ..Default::default()
            },
            None,
        )
    })
    .unwrap();
    assert_eq!(
        tier(&db, "linked"),
        2,
        "a tier 2 tag wins, whatever its case"
    );
}

#[test]
fn off_and_shadow_gate_nothing() {
    for mode in [VerificationMode::Off, VerificationMode::Shadow] {
        let (db, initiative) = setup();
        policy(&db, mode);
        in_review(&db, &initiative, "tk-1", "alpha");
        let done = update_as(&db, "tk-1", STATUS_COMPLETE, &WriteCtx::default()).unwrap();
        assert_eq!(done.status, STATUS_COMPLETE, "{mode:?}");
        assert!(
            db.with_conn(|c| latest_verdict(c, "tk-1"))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn enforced_completion_needs_a_verdict_or_the_owner() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");

    let refused = update_as(&db, "tk-1", STATUS_COMPLETE, &WriteCtx::default()).unwrap_err();
    assert!(needs_verification(&refused), "{refused}");
    assert!(refused.to_string().contains("ready_for_review"));
    let named = update_as(
        &db,
        "tk-1",
        STATUS_COMPLETE,
        &WriteCtx {
            actor: Some("alpha"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(needs_verification(&named), "a name is not authority");
    assert_eq!(get_one(&db, "tk-1").status, STATUS_READY_FOR_REVIEW);
    assert!(
        update_as(&db, "tk-1", STATUS_IN_PROGRESS, &WriteCtx::default()).is_ok(),
        "every other edge stays as free as before"
    );
}

#[test]
fn the_owner_can_always_complete_and_it_is_recorded_as_such() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");
    let owner = WriteCtx {
        actor: Some(WEB_ACTOR),
        owner: true,
        ..Default::default()
    };
    assert_eq!(
        update_as(&db, "tk-1", STATUS_COMPLETE, &owner)
            .unwrap()
            .status,
        STATUS_COMPLETE
    );
    let v = db
        .with_conn(|c| latest_verdict(c, "tk-1"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (v.verdict.as_str(), v.level.as_str()),
        (VERDICT_PASS, LEVEL_OWNER)
    );
    assert_eq!(v.verifier, WEB_ACTOR);
    assert!(v.applied);

    // Shadow mode records an owner completion too, so the first accuracy numbers have a baseline.
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Shadow);
    in_review(&db, &initiative, "tk-2", "alpha");
    update_as(&db, "tk-2", STATUS_COMPLETE, &owner).unwrap();
    assert!(
        db.with_conn(|c| latest_verdict(c, "tk-2"))
            .unwrap()
            .is_some()
    );

    // Off records nothing: a fresh install is unchanged.
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-3", "alpha");
    update_as(&db, "tk-3", STATUS_COMPLETE, &owner).unwrap();
    assert!(
        db.with_conn(|c| latest_verdict(c, "tk-3"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn only_the_tiers_named_are_gated() {
    let (db, initiative) = setup();
    policy_with(
        &db,
        VerificationConfig {
            mode: VerificationMode::Enforce,
            enforce_tiers: vec![1, 2],
            ..Default::default()
        },
    );
    in_review(&db, &initiative, "docs", "alpha");
    assert_eq!(
        update_as(&db, "docs", STATUS_COMPLETE, &WriteCtx::default())
            .unwrap()
            .status,
        STATUS_COMPLETE,
        "tier 0 is not staged in yet"
    );
    in_review(&db, &initiative, "code", "alpha");
    code_task(&db, "code");
    let refused = update_as(&db, "code", STATUS_COMPLETE, &WriteCtx::default()).unwrap_err();
    assert!(needs_verification(&refused));
}

#[test]
fn a_passing_applied_verdict_completes_the_task() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");
    let verdict = passing_verdict(&db, "tk-1", "verify:run-1");
    let ctx = WriteCtx {
        actor: Some("verify:run-1"),
        verdict_id: Some(&verdict.id),
        ..Default::default()
    };
    let done = update_as(&db, "tk-1", STATUS_COMPLETE, &ctx).unwrap();
    assert_eq!(done.status, STATUS_COMPLETE);
    let events = db.with_conn(|c| list_task_events(c, "tk-1")).unwrap();
    assert_eq!(
        events.last().unwrap().actor.as_deref(),
        Some("verify:run-1")
    );
    assert!(
        get_one(&db, "tk-1").completed_at.is_some(),
        "completed_at is stamped as for any completion"
    );
}

#[test]
fn a_verdict_that_does_not_fit_the_task_cannot_be_used() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");
    in_review(&db, &initiative, "tk-2", "alpha");
    let other = passing_verdict(&db, "tk-2", "verify:run-1");
    let mismatched = WriteCtx {
        verdict_id: Some(&other.id),
        ..Default::default()
    };
    let err = update_as(&db, "tk-1", STATUS_COMPLETE, &mismatched).unwrap_err();
    assert!(
        !needs_verification(&err) && err.to_string().contains("does not allow"),
        "{err}"
    );

    // A failing verdict, a verdict not applied, and an unknown id all fail the same way.
    let failed = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    verdict: VERDICT_FAIL,
                    level: LEVEL_NONE,
                    applied: false,
                    reasons: "tests do not pass",
                    ..pass_for("tk-1", "verify:run-1", &json!([]), &json!([]))
                },
            )
        })
        .unwrap();
    let (obs, crit) = good_evidence();
    let unapplied = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    applied: false,
                    ..pass_for("tk-1", "verify:run-1", &obs, &crit)
                },
            )
        })
        .unwrap();
    for id in [failed.id.as_str(), unapplied.id.as_str(), "vd-nope"] {
        let ctx = WriteCtx {
            verdict_id: Some(id),
            ..Default::default()
        };
        assert!(
            update_as(&db, "tk-1", STATUS_COMPLETE, &ctx).is_err(),
            "{id}"
        );
    }
    assert_eq!(get_one(&db, "tk-1").status, STATUS_READY_FOR_REVIEW);
}

#[test]
fn a_verdict_for_an_earlier_submission_cannot_complete_reworked_work() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");
    let verdict = passing_verdict(&db, "tk-1", "verify:run-1");
    // The work was sent back and submitted again right after the verdict was made, inside the
    // same second: only the submission id, never the clock, can tell the two apart.
    move_to(&db, "tk-1", STATUS_IN_PROGRESS).unwrap();
    set_status_as(&db, "tk-1", STATUS_READY_FOR_REVIEW, "alpha");
    let ctx = WriteCtx {
        verdict_id: Some(&verdict.id),
        ..Default::default()
    };
    let err = update_as(&db, "tk-1", STATUS_COMPLETE, &ctx).unwrap_err();
    assert!(
        err.to_string()
            .contains("about the submission it is in now"),
        "{err}"
    );
}

#[test]
fn a_worker_cannot_verify_its_own_task_and_an_owner_verdict_cannot_be_supplied() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let (obs, crit) = good_evidence();
    let err = db
        .with_conn(|c| record_verdict(c, &pass_for("tk-1", "Alpha", &obs, &crit)))
        .unwrap_err();
    assert!(err.to_string().contains("cannot verify"), "{err}");

    // A lease holder is a worker even if it never moved the status.
    make(&db, "tk-2", &initiative, None).unwrap();
    claim_as(&db, "tk-2", "beta").unwrap();
    let err = db
        .with_conn(|c| record_verdict(c, &pass_for("tk-2", "beta", &obs, &crit)))
        .unwrap_err();
    assert!(err.to_string().contains("cannot verify"), "{err}");

    let err = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    level: LEVEL_OWNER,
                    ..pass_for("tk-1", "someone", &obs, &crit)
                },
            )
        })
        .unwrap_err();
    assert!(err.to_string().contains("owner verdict"), "{err}");
}

#[test]
fn a_pass_must_rest_on_recorded_facts() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let verdict = |obs: serde_json::Value, crit: serde_json::Value, tier: u8| {
        db.with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    tier,
                    ..pass_for("tk-1", "verify:run-1", &obs, &crit)
                },
            )
        })
    };
    let obs = json!([{"id": "o1", "fact": "x"}]);

    let err = verdict(json!([]), json!([]), 0).unwrap_err().to_string();
    assert!(err.contains("at least one recorded observation"), "{err}");

    let uncited = json!([{"text": "works", "result": "pass", "observation_ids": []}]);
    let err = verdict(obs.clone(), uncited, 1).unwrap_err().to_string();
    assert!(err.contains("without citing"), "{err}");

    let unknown = json!([{"text": "works", "result": "pass", "observation_ids": ["o9"]}]);
    let err = verdict(obs.clone(), unknown, 1).unwrap_err().to_string();
    assert!(err.contains("never recorded"), "{err}");

    let unmet = json!([{"text": "works", "result": "fail", "observation_ids": ["o1"]}]);
    let err = verdict(obs.clone(), unmet, 1).unwrap_err().to_string();
    assert!(err.contains("did not pass"), "{err}");

    let err = verdict(obs.clone(), json!([]), 1).unwrap_err().to_string();
    assert!(err.contains("needs criteria"), "{err}");
    assert!(
        verdict(obs.clone(), json!([]), 0).is_ok(),
        "only tier 0 may be assessed with no criteria"
    );

    let twice = json!([{"id": "o1", "fact": "x"}, {"id": "o1", "fact": "y"}]);
    assert!(
        verdict(twice, json!([]), 0)
            .unwrap_err()
            .to_string()
            .contains("used twice")
    );
}

#[test]
fn a_pass_with_no_level_or_a_fail_that_claims_to_be_applied_is_refused() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let (obs, crit) = good_evidence();
    let err = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    level: LEVEL_NONE,
                    ..pass_for("tk-1", "v", &obs, &crit)
                },
            )
        })
        .unwrap_err();
    assert!(err.to_string().contains("level none cannot pass"), "{err}");
    let err = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    verdict: VERDICT_FAIL,
                    level: LEVEL_NONE,
                    applied: true,
                    ..pass_for("tk-1", "v", &obs, &crit)
                },
            )
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("only a pass can be applied"),
        "{err}"
    );
    // A fail needs no evidence: saying no costs nothing.
    assert!(
        db.with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    verdict: VERDICT_FAIL,
                    level: LEVEL_NONE,
                    applied: false,
                    ..pass_for("tk-1", "v", &json!([]), &json!([]))
                },
            )
        })
        .is_ok()
    );
}

#[test]
fn verdicts_are_append_only_and_only_a_purge_removes_them() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let verdict = passing_verdict(&db, "tk-1", "verify:run-1");
    let update = db.with_conn(|c| {
        c.execute(
            "UPDATE task_verdicts SET verdict = 'fail' WHERE id = ?1",
            params![verdict.id],
        )?;
        Ok(())
    });
    assert!(update.unwrap_err().to_string().contains("append-only"));
    let delete = db.with_conn(|c| {
        c.execute(
            "DELETE FROM task_verdicts WHERE id = ?1",
            params![verdict.id],
        )?;
        Ok(())
    });
    assert!(delete.unwrap_err().to_string().contains("append-only"));

    db.with_conn(|c| archive_task(c, "tk-1", false, "t"))
        .unwrap();
    db.with_conn(|c| purge_task(c, "tk-1", "t")).unwrap();
    let left: i64 = db
        .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM task_verdicts", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(left, 0, "purging an archived task removes what it holds");
}

#[test]
fn the_latest_verdict_and_the_list_are_newest_first() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let first = passing_verdict(&db, "tk-1", "verify:run-1");
    let second = passing_verdict(&db, "tk-1", "verify:run-2");
    assert_eq!(
        db.with_conn(|c| latest_verdict(c, "tk-1"))
            .unwrap()
            .unwrap()
            .id,
        second.id
    );
    let listed = db.with_conn(|c| list_verdicts(c, "tk-1", 10)).unwrap();
    assert_eq!(
        listed.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        [second.id.as_str(), first.id.as_str()]
    );
    assert_eq!(listed[0].observations[0]["id"], "o1");
}

#[test]
fn releasing_as_complete_under_enforcement_hands_over_for_review_instead() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    make(&db, "tk-1", &initiative, None).unwrap();
    assign(&db, "tk-1", &["hq"]);
    let claimed = claim_as(&db, "tk-1", "alpha").unwrap();
    let done = db
        .with_conn(|c| release(c, &claimed.token, Some(STATUS_COMPLETE), "all done", TTL))
        .unwrap();
    assert_eq!(done.task.status, STATUS_READY_FOR_REVIEW);
    assert!(done.verification_queued && done.status_applied);
    let note = db
        .with_conn(|c| list_comments(c, "tk-1"))
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        note.body.contains("ready_for_review") && note.body.contains("passes verification"),
        "{}",
        note.body
    );

    // With verification off the same release completes the task, as it always did.
    let (db, initiative) = setup();
    make(&db, "tk-2", &initiative, None).unwrap();
    let claimed = db
        .with_conn(|c| claim(c, "tk-2", &who("beta"), TTL, false))
        .unwrap();
    let done = db
        .with_conn(|c| release(c, &claimed.token, Some(STATUS_COMPLETE), "", TTL))
        .unwrap();
    assert_eq!(done.task.status, STATUS_COMPLETE);
    assert!(!done.verification_queued);
}

#[test]
fn completion_requires_verification_follows_mode_and_tier() {
    let (db, initiative) = setup();
    make(&db, "tk-1", &initiative, None).unwrap();
    let task = get_one(&db, "tk-1");
    let asks = |db: &Database| {
        db.with_conn(|c| completion_requires_verification(c, &task))
            .unwrap()
    };
    assert!(!asks(&db), "off");
    policy(&db, VerificationMode::Shadow);
    assert!(!asks(&db), "shadow");
    policy(&db, VerificationMode::Enforce);
    assert!(asks(&db), "enforce, every tier");
    policy_with(
        &db,
        VerificationConfig {
            mode: VerificationMode::Enforce,
            enforce_tiers: vec![2],
            ..Default::default()
        },
    );
    assert!(!asks(&db), "enforce, a tier this task is not in");
}

#[test]
fn moving_back_to_work_after_a_verdict_is_not_gated() {
    let (db, initiative) = setup();
    policy(&db, VerificationMode::Enforce);
    in_review(&db, &initiative, "tk-1", "alpha");
    let verdict = passing_verdict(&db, "tk-1", "verify:run-1");
    let ctx = WriteCtx {
        verdict_id: Some(&verdict.id),
        ..Default::default()
    };
    update_as(&db, "tk-1", STATUS_COMPLETE, &ctx).unwrap();
    // Reopening is free, and completing again needs a fresh verdict.
    move_to(&db, "tk-1", STATUS_IN_PROGRESS).unwrap();
    set_status_as(&db, "tk-1", STATUS_READY_FOR_REVIEW, "alpha");
    let err = update_as(&db, "tk-1", STATUS_COMPLETE, &WriteCtx::default()).unwrap_err();
    assert!(needs_verification(&err));
}

#[test]
fn the_queue_lists_only_tasks_without_a_verdict_for_their_current_submission() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig::default();
    in_review(&db, &initiative, "fresh", "alpha");
    in_review(&db, &initiative, "done", "alpha");
    passing_verdict(&db, "done", "verify:run-1");
    make(&db, "idle", &initiative, None).unwrap();

    let ids = |db: &Database| -> Vec<String> {
        db.with_conn(|c| verification_candidates(c, &cfg, chrono::Utc::now().timestamp()))
            .unwrap()
            .into_iter()
            .map(|c| c.task.id)
            .collect()
    };
    assert_eq!(
        ids(&db),
        ["fresh"],
        "a verdict covers its submission; to_do is not in review"
    );

    // Sent back and submitted again, in the same second: the old verdict no longer covers it.
    move_to(&db, "done", STATUS_IN_PROGRESS).unwrap();
    set_status_as(&db, "done", STATUS_READY_FOR_REVIEW, "alpha");
    assert_eq!(ids(&db).len(), 2);
}

#[test]
fn the_queue_puts_what_others_wait_on_first_then_tier_two_then_the_oldest() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig::default();
    for id in ["old", "newer", "risky", "blocker"] {
        in_review(&db, &initiative, id, "alpha");
    }
    // Spread the submissions in time: old < newer < risky < blocker.
    for (id, hours) in [("old", 4), ("newer", 3), ("risky", 2), ("blocker", 1)] {
        db.with_conn(|c| {
            c.execute(
                "UPDATE task_events SET occurred_at = datetime('now', ?1) \
                 WHERE task_id = ?2 AND event_type = 'entered_ready_for_review'",
                params![format!("-{hours} hours"), id],
            )?;
            Ok(())
        })
        .unwrap();
    }
    db.with_conn(|c| {
        update_task(
            c,
            "risky",
            &TaskPatch {
                tags: Some(vec!["production".into()]),
                ..Default::default()
            },
            None,
        )
    })
    .unwrap();
    make(&db, "waiting", &initiative, None).unwrap();
    db.with_conn(|c| add_dependency(c, "waiting", "blocker", "t"))
        .unwrap();

    let order: Vec<String> = db
        .with_conn(|c| verification_candidates(c, &cfg, chrono::Utc::now().timestamp()))
        .unwrap()
        .into_iter()
        .map(|c| c.task.id)
        .collect();
    assert_eq!(order, ["blocker", "risky", "old", "newer"]);
}

#[test]
fn a_submission_has_a_marker_that_changes_when_it_is_submitted_again() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let first = db.with_conn(|c| review_marker(c, "tk-1")).unwrap().unwrap();
    set_status_as(&db, "tk-1", STATUS_IN_PROGRESS, "alpha");
    set_status_as(&db, "tk-1", STATUS_READY_FOR_REVIEW, "alpha");
    let second = db.with_conn(|c| review_marker(c, "tk-1")).unwrap().unwrap();
    assert!(second > first);
    make(&db, "never", &initiative, None).unwrap();
    assert_eq!(db.with_conn(|c| review_marker(c, "never")).unwrap(), None);
}

#[test]
fn reviewer_spend_is_read_from_the_ledger_by_its_origin() {
    let (db, _) = setup();
    assert_eq!(db.with_conn(reviewer_spend_this_month).unwrap(), 0.0);
    let insert = |db: &Database, origin: &str, cost: f64, age_secs: i64| {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO task_outcomes (session_id, turn_idx, model, provider, task_hint, latency_ms, \
                     cost_usd, success, recorded_at, origin) \
                 VALUES ('s', 0, 'm', 'p', 'h', 1, ?1, 1, strftime('%s','now') - ?2, ?3)",
                params![cost, age_secs, origin],
            )?;
            Ok(())
        })
        .unwrap();
    };
    insert(&db, "verification", 1.25, 0);
    insert(&db, "verification", 0.75, 0);
    insert(&db, "chat", 10.0, 0);
    insert(&db, "verification", 99.0, 400 * 24 * 3600);
    assert_eq!(
        db.with_conn(reviewer_spend_this_month).unwrap(),
        2.0,
        "this month's reviewer calls only: not chat, and not last year"
    );
}

#[test]
fn reviewer_runs_are_counted_from_the_verdicts_that_recorded_one() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    assert_eq!(db.with_conn(reviewer_runs_recorded).unwrap(), 0);
    let obs = json!([
        {"id": "o1", "kind": "reviewer_run", "fact": "a reviewer ran"},
    ]);
    db.with_conn(|c| {
        record_verdict(
            c,
            &NewVerdict {
                verdict: VERDICT_INCONCLUSIVE,
                level: LEVEL_NONE,
                applied: false,
                ..pass_for("tk-1", "verify:run-1", &obs, &json!([]))
            },
        )
    })
    .unwrap();
    assert_eq!(db.with_conn(reviewer_runs_recorded).unwrap(), 1);
}

// ---- waits and rework ----

fn failing_verdict(db: &Database, task: &str) -> Verdict {
    let (obs, crit) = good_evidence();
    db.with_conn(|c| {
        record_verdict(
            c,
            &NewVerdict {
                verdict: VERDICT_FAIL,
                level: LEVEL_NONE,
                applied: false,
                reasons: "the tests do not cover closed jobs",
                waiting_for: Some(WAIT_WORKER),
                ..pass_for(task, "verify:run-1", &obs, &crit)
            },
        )
    })
    .unwrap()
}

fn resubmit(db: &Database, id: &str) {
    move_to(db, id, STATUS_IN_PROGRESS).unwrap();
    set_status_as(db, id, STATUS_READY_FOR_REVIEW, "alpha");
}

#[test]
fn a_wait_hides_a_task_until_it_is_due_and_counts_looks_at_one_submission() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig::default();
    in_review(&db, &initiative, "tk-1", "alpha");
    let queued = |at: i64| {
        db.with_conn(|c| verification_candidates(c, &cfg, at))
            .unwrap()
            .len()
    };
    let first = db
        .with_conn(|c| record_wait(c, "tk-1", WAIT_CI, "checks are running", 300, 1_000))
        .unwrap();
    assert_eq!((first.tries, first.retry_at), (1, 1_300));
    assert_eq!(queued(1_100), 0, "waiting out its retry");
    assert_eq!(queued(1_300), 1, "due");

    let second = db
        .with_conn(|c| record_wait(c, "tk-1", WAIT_BUDGET, "ceiling used", 1_800, 1_300))
        .unwrap();
    assert_eq!(
        (second.tries, second.waiting_for.as_str()),
        (2, WAIT_BUDGET)
    );
    assert_eq!(
        db.with_conn(|c| current_wait(c, "tk-1")).unwrap().unwrap(),
        second
    );
}

#[test]
fn a_wait_for_an_earlier_submission_is_not_a_wait_for_this_one() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig::default();
    in_review(&db, &initiative, "tk-1", "alpha");
    db.with_conn(|c| record_wait(c, "tk-1", WAIT_CI, "checks are running", 10_000, 0))
        .unwrap();
    resubmit(&db, "tk-1");
    assert!(db.with_conn(|c| current_wait(c, "tk-1")).unwrap().is_none());
    assert_eq!(
        db.with_conn(|c| verification_candidates(c, &cfg, 1))
            .unwrap()
            .len(),
        1,
        "the new submission is looked at at once"
    );
    let again = db
        .with_conn(|c| record_wait(c, "tk-1", WAIT_CI, "checks are running", 60, 1))
        .unwrap();
    assert_eq!(again.tries, 1, "a new submission starts counting over");
}

#[test]
fn a_verdict_names_who_acts_next_only_from_the_known_set_and_never_for_a_pass() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let (obs, crit) = good_evidence();
    let record = |waiting_for: Option<&'static str>, verdict: &'static str| {
        db.with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    verdict,
                    waiting_for,
                    applied: false,
                    ..pass_for("tk-1", "verify:run-1", &obs, &crit)
                },
            )
        })
    };
    assert!(record(Some("the moon"), VERDICT_FAIL).is_err());
    let err = record(Some(WAIT_WORKER), VERDICT_PASS).unwrap_err();
    assert!(err.to_string().contains("waits on no one"), "{err}");
    assert_eq!(
        record(Some(WAIT_OWNER), VERDICT_FAIL)
            .unwrap()
            .waiting_for
            .as_deref(),
        Some("owner")
    );
}

#[test]
fn a_failed_task_goes_back_to_to_do_keeping_its_worker_with_the_findings_to_resume_from() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    assign(&db, "tk-1", &["alpha"]);
    failing_verdict(&db, "tk-1");
    let done = db
        .with_conn(|c| {
            send_back(
                c,
                "tk-1",
                "verify:run-1",
                "the tests do not cover closed jobs",
                3,
            )
        })
        .unwrap();
    assert_eq!(done, Reworked::Returned { rejections: 1 });

    let task = get_one(&db, "tk-1");
    assert_eq!(task.status, STATUS_TO_DO);
    assert_eq!(task.assignees, ["alpha"]);
    let checkpoint = db
        .with_conn(|c| latest_checkpoint(c, "tk-1"))
        .unwrap()
        .unwrap();
    assert!(checkpoint.summary.contains("closed jobs"), "{checkpoint:?}");
    assert!(checkpoint.next_step.contains("ready_for_review"));
    let comments = db.with_conn(|c| list_comments(c, "tk-1")).unwrap();
    let note = &comments.last().unwrap().body;
    assert!(note.contains("return 1 of 3"), "{note}");
    assert!(
        note.contains("> the tests do not cover closed jobs"),
        "{note}"
    );
    assert_eq!(db.with_conn(|c| rejections(c, "tk-1")).unwrap(), 1);
}

#[test]
fn the_third_return_is_one_exception_for_the_owner_not_another_bounce() {
    let (db, initiative) = setup();
    in_review(&db, &initiative, "tk-1", "alpha");
    let mut outcomes = Vec::new();
    for round in 0..3 {
        if round > 0 {
            resubmit(&db, "tk-1");
        }
        failing_verdict(&db, "tk-1");
        outcomes.push(
            db.with_conn(|c| send_back(c, "tk-1", "verify:run-1", "still failing", 3))
                .unwrap(),
        );
    }
    assert_eq!(
        outcomes,
        [
            Reworked::Returned { rejections: 1 },
            Reworked::Returned { rejections: 2 },
            Reworked::Escalated { rejections: 3 },
        ]
    );
    let task = get_one(&db, "tk-1");
    assert_eq!(task.status, STATUS_BLOCKED);
    assert_eq!(task.waiting_on.as_deref(), Some("owner"));
    assert!(task.blocked_reason.unwrap().contains("3 times"));
    let wait = db.with_conn(|c| current_wait(c, "tk-1")).unwrap().unwrap();
    assert_eq!(wait.waiting_for, WAIT_OWNER);
}

#[test]
fn only_a_task_waiting_for_review_can_be_sent_back() {
    let (db, initiative) = setup();
    make(&db, "tk-1", &initiative, None).unwrap();
    let err = db
        .with_conn(|c| send_back(c, "tk-1", "verify:run-1", "x", 3))
        .unwrap_err();
    assert!(err.to_string().contains("not waiting for review"), "{err}");
}
