use super::tests::{get_one, make, move_to, setup};
use super::*;
use crate::Database;
use hq_core::config::{AutonomyGate, VerificationConfig};
use serde_json::json;

fn cfg() -> VerificationConfig {
    VerificationConfig {
        autonomy_min_audits: 4,
        autonomy_min_canaries: 4,
        ..Default::default()
    }
}

/// A task HQ completed: submitted by a worker, passed by a verifier under `config`.
pub(super) fn verified(
    db: &Database,
    initiative: &str,
    id: &str,
    tier: u8,
    config: &str,
) -> Verdict {
    make(db, id, initiative, None).unwrap();
    move_to(db, id, STATUS_IN_PROGRESS).unwrap();
    move_to(db, id, STATUS_READY_FOR_REVIEW).unwrap();
    let obs = json!([{"id": "o1", "source": "github", "fact": "checks passed"}]);
    let crit = json!([{"text": "works", "result": "pass", "observation_ids": ["o1"]}]);
    let v = db
        .with_conn(|c| {
            record_verdict(
                c,
                &NewVerdict {
                    task_id: id,
                    artifact: "pr:example/repo#1@abc",
                    contract_hash: "h",
                    tier,
                    verdict: VERDICT_PASS,
                    level: LEVEL_MACHINE_PEER,
                    verifier: "verify:run-1",
                    independence: "different_session",
                    observations: &obs,
                    criteria: &crit,
                    reasons: "",
                    applied: true,
                    canary: false,
                    submission: None,
                    waiting_for: None,
                    verifier_config: config,
                },
            )
        })
        .unwrap();
    // Completed the way the verifier does it: presenting the verdict, which enforcement requires.
    db.with_conn(|c| {
        update_task_as(
            c,
            id,
            &TaskPatch {
                status: Some(STATUS_COMPLETE.into()),
                ..Default::default()
            },
            None,
            &WriteCtx {
                actor: Some("verify:test"),
                verdict_id: Some(&v.id),
                ..Default::default()
            },
        )
    })
    .unwrap();
    v
}

pub(super) fn answered(db: &Database, id: &str, agrees: bool) {
    db.with_conn(|c| answer_audit(c, id, agrees, "", "owner"))
        .unwrap();
}

#[test]
fn a_rate_seen_in_few_audits_is_bounded_honestly() {
    assert!(
        (wilson_upper(0, 30) - 0.0827).abs() < 0.001,
        "{}",
        wilson_upper(0, 30)
    );
    assert!(wilson_upper(1, 30) > 0.10);
    assert!(wilson_upper(0, 10) > 0.2, "ten clean audits prove little");
    assert_eq!(wilson_upper(0, 0), 1.0, "no data bounds nothing");
}

#[test]
fn audits_are_drawn_once_from_completed_verified_tasks_only() {
    let (db, initiative) = setup();
    for n in 0..5 {
        verified(&db, &initiative, &format!("tk-{n}"), 1, "m");
    }
    // Not complete, so not auditable; and a task the owner completed is not the verifier's work.
    make(&db, "open", &initiative, None).unwrap();
    let first = db
        .with_conn(|c| select_audits(c, 3, AUDIT_PULL, 1))
        .unwrap();
    assert_eq!(first.len(), 3);
    let ids: std::collections::HashSet<_> = first.iter().map(|a| a.task_id.clone()).collect();
    assert_eq!(ids.len(), 3, "no task twice");
    let rest = db
        .with_conn(|c| select_audits(c, 10, AUDIT_PULL, 2))
        .unwrap();
    assert_eq!(rest.len(), 2, "only what has not been audited");
    assert!(
        db.with_conn(|c| select_audits(c, 10, AUDIT_PULL, 3))
            .unwrap()
            .is_empty()
    );
    assert!(db.with_conn(|c| select_audits(c, 1, "bogus", 3)).is_err());
}

#[test]
fn the_draw_favours_the_verifier_configuration_with_the_least_audit_data() {
    let (db, initiative) = setup();
    // Ten answered audits of configuration "old", one candidate left in each configuration.
    for n in 0..10 {
        verified(&db, &initiative, &format!("old-{n}"), 1, "old");
    }
    let drawn = db
        .with_conn(|c| select_audits(c, 10, AUDIT_PULL, 5))
        .unwrap();
    for a in &drawn {
        answered(&db, &a.id, true);
    }
    verified(&db, &initiative, "old-left", 1, "old");
    verified(&db, &initiative, "fresh-left", 1, "fresh");
    let mut fresh = 0;
    for seed in 0..40 {
        let pick = db
            .with_conn(|c| select_audits(c, 1, AUDIT_PULL, seed))
            .unwrap();
        if pick[0].verifier_config == "fresh" {
            fresh += 1;
        }
        db.with_conn(|c| {
            c.execute("DELETE FROM task_audits WHERE id = ?1", params![pick[0].id])?;
            Ok(())
        })
        .unwrap();
    }
    assert!(
        fresh >= 30,
        "the unaudited configuration was drawn {fresh} times of 40"
    );
}

#[test]
fn a_disagreement_reopens_the_task_and_an_audit_is_answered_once() {
    let (db, initiative) = setup();
    verified(&db, &initiative, "tk-1", 1, "m");
    verified(&db, &initiative, "tk-2", 1, "m");
    let audits = db
        .with_conn(|c| select_audits(c, 2, AUDIT_PULL, 1))
        .unwrap();
    for audit in &audits {
        if audit.task_id == "tk-1" {
            db.with_conn(|c| answer_audit(c, &audit.id, false, "the test is empty", "owner"))
                .unwrap();
        } else {
            answered(&db, &audit.id, true);
        }
    }
    assert_eq!(get_one(&db, "tk-1").status, STATUS_TO_DO);
    assert_eq!(get_one(&db, "tk-2").status, STATUS_COMPLETE);
    let comments = db.with_conn(|c| list_comments(c, "tk-1")).unwrap();
    assert!(comments.last().unwrap().body.contains("the test is empty"));
    assert!(
        db.with_conn(|c| answer_audit(c, &audits[0].id, true, "", "owner"))
            .is_err()
    );
    assert!(db.with_conn(|c| open_audits(c, 10)).unwrap().is_empty());
}

#[test]
fn the_weekly_floor_is_pushed_once_and_never_beyond_what_exists() {
    let (db, initiative) = setup();
    for n in 0..4 {
        verified(&db, &initiative, &format!("tk-{n}"), 0, "m");
    }
    let c = VerificationConfig {
        audit_min_per_week: 3,
        audit_rate: 0.05,
        ..cfg()
    };
    assert_eq!(db.with_conn(|x| weekly_audit_target(x, &c)).unwrap(), 3);
    let first = db.with_conn(|x| push_weekly_audits(x, &c, 1)).unwrap();
    assert_eq!(first.len(), 3);
    assert!(first.iter().all(|a| a.source == AUDIT_PUSH));
    let again = db.with_conn(|x| push_weekly_audits(x, &c, 2)).unwrap();
    assert!(again.is_empty(), "the week's floor is met");
}

#[test]
fn recent_disagreements_raise_the_week_s_target() {
    let (db, initiative) = setup();
    for n in 0..40 {
        verified(&db, &initiative, &format!("tk-{n}"), 0, "m");
    }
    let c = VerificationConfig {
        audit_min_per_week: 1,
        audit_rate: 0.05,
        ..cfg()
    };
    let base = db.with_conn(|x| weekly_audit_target(x, &c)).unwrap();
    assert_eq!(base, 2, "5% of 40");
    let audits = db
        .with_conn(|x| select_audits(x, 1, AUDIT_PULL, 1))
        .unwrap();
    db.with_conn(|x| answer_audit(x, &audits[0].id, false, "", "owner"))
        .unwrap();
    let raised = db.with_conn(|x| weekly_audit_target(x, &c)).unwrap();
    assert!(raised > base, "{raised} > {base}");
}

#[test]
fn calibration_says_insufficient_data_with_counts_until_there_is_enough() {
    let (db, _) = setup();
    let cal = db.with_conn(|c| calibration(c, &cfg())).unwrap();
    for tier in &cal.tiers {
        assert_eq!(tier.state, Autonomy::Insufficient);
        assert!(tier.why[0].contains("0 of 4 audits"), "{:?}", tier.why);
        assert!(tier.disagreement_upper.is_none(), "no rate invented");
    }
    assert!(cal.canaries.catch_rate.is_none());
}

fn run_canaries(db: &Database, key: &str, defects: (usize, usize), controls: usize) {
    db.with_conn(|c| {
        for n in 0..defects.0 {
            let got = if n < defects.1 { "fail" } else { "pass" };
            record_canary(c, "unmet_criterion", "fail", got, key, "")?;
        }
        for _ in 0..controls {
            record_canary(c, "clear_pass", "pass", "pass", key, "")?;
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn autonomy_is_earned_from_audits_and_canaries_and_withdrawn_when_they_say_so() {
    let (db, initiative) = setup();
    // Enough clean audits for the error rate to be bounded under the limit.
    let c = VerificationConfig {
        autonomy_min_audits: 30,
        ..cfg()
    };
    let key = current_config_key(&c);
    for n in 0..30 {
        verified(&db, &initiative, &format!("tk-{n}"), 1, &key);
    }
    let audits = db
        .with_conn(|x| select_audits(x, 30, AUDIT_PULL, 1))
        .unwrap();
    for a in &audits {
        answered(&db, &a.id, true);
    }
    run_canaries(&db, &key, (4, 4), 2);
    let cal = db.with_conn(|x| calibration(x, &c)).unwrap();
    let tier1 = &cal.tiers[1];
    assert_eq!(tier1.state, Autonomy::Earned, "{:?}", tier1.why);
    assert_eq!(cal.canaries.catch_rate, Some(1.0));
    assert_eq!(cal.tiers[1].audits, 30);
    assert_eq!(
        cal.tiers[0].state,
        Autonomy::Insufficient,
        "tier 0 has no audits"
    );

    // The reviewer starts approving planted defects: autonomy is withdrawn.
    run_canaries(&db, &key, (6, 0), 0);
    let cal = db.with_conn(|x| calibration(x, &c)).unwrap();
    assert_eq!(cal.tiers[1].state, Autonomy::Withdrawn);
    assert!(
        cal.tiers[1]
            .why
            .iter()
            .any(|w| w.contains("caught 4 of 10")),
        "{:?}",
        cal.tiers[1].why
    );
}

#[test]
fn the_gate_holds_back_only_what_its_mode_says() {
    let (db, initiative) = setup();
    let mut c = cfg();
    let key = current_config_key(&c);
    // Withdrawn: the reviewer passes every planted defect.
    run_canaries(&db, &key, (6, 0), 0);
    for (mode, blocked) in [
        (AutonomyGate::Off, false),
        (AutonomyGate::Demote, true),
        (AutonomyGate::Require, true),
    ] {
        c.autonomy_gate = mode;
        assert_eq!(
            db.with_conn(|x| autonomy_block(x, &c, 1))
                .unwrap()
                .is_some(),
            blocked,
            "{mode:?}"
        );
    }
    // Insufficient only holds back under `require`.
    let (db2, _) = setup();
    for (mode, blocked) in [
        (AutonomyGate::Off, false),
        (AutonomyGate::Demote, false),
        (AutonomyGate::Require, true),
    ] {
        c.autonomy_gate = mode;
        assert_eq!(
            db2.with_conn(|x| autonomy_block(x, &c, 1))
                .unwrap()
                .is_some(),
            blocked,
            "{mode:?}"
        );
    }
    let _ = initiative;
}

#[test]
fn a_reopened_task_and_a_regression_follow_up_each_count_as_an_escape() {
    let (db, initiative) = setup();
    let c = cfg();
    let key = current_config_key(&c);
    for id in ["reopened", "regressed", "fine"] {
        verified(&db, &initiative, id, 1, &key);
    }
    move_to(&db, "reopened", STATUS_IN_PROGRESS).unwrap();
    make(&db, "follow-up", &initiative, None).unwrap();
    db.with_conn(|x| {
        update_task(
            x,
            "follow-up",
            &TaskPatch {
                tags: Some(vec!["regression".into()]),
                ..Default::default()
            },
            None,
        )?;
        add_task_link(x, "follow-up", "task", "regressed", "", None, "t")
    })
    .unwrap();
    assert_eq!(db.with_conn(|x| escapes(x, 1, &key)).unwrap(), 2);
    assert_eq!(
        db.with_conn(|x| escapes(x, 0, &key)).unwrap(),
        0,
        "another tier"
    );
}

/// A verdict a sweep recorded and left unapplied: the task stays in review.
fn recorded_in_review(db: &Database, id: &str, tier: u8, verdict: &str, config: &str) -> Verdict {
    let obs = json!([{"id": "o1", "source": "github", "fact": "checks passed"}]);
    let crit = json!([{"text": "works", "result": verdict, "observation_ids": ["o1"]}]);
    db.with_conn(|c| {
        record_verdict(
            c,
            &NewVerdict {
                task_id: id,
                artifact: "pr:example/repo#1@abc",
                contract_hash: "h",
                tier,
                verdict,
                level: if verdict == VERDICT_PASS {
                    LEVEL_MACHINE_PEER
                } else {
                    LEVEL_NONE
                },
                verifier: "verify:run-1",
                independence: "different_session",
                observations: &obs,
                criteria: &crit,
                reasons: "recorded by a sweep",
                applied: verdict == VERDICT_PASS,
                canary: false,
                submission: None,
                waiting_for: (verdict != VERDICT_PASS).then_some("worker"),
                verifier_config: config,
            },
        )
    })
    .unwrap()
}

#[test]
fn a_pass_that_never_completed_the_task_is_not_an_escape() {
    let (db, initiative) = setup();
    let c = cfg();
    let key = current_config_key(&c);
    // Three passes a sweep recorded and has not applied: the tasks are still in review.
    for n in 0..3 {
        let id = format!("swept-{n}");
        waiting(&db, &initiative, &id);
        recorded_in_review(&db, &id, 1, VERDICT_PASS, &key);
    }
    assert_eq!(
        db.with_conn(|x| escapes(x, 1, &key)).unwrap(),
        0,
        "nothing was completed, so nothing came back"
    );
    // One that was completed and then reopened is.
    verified(&db, &initiative, "reopened", 1, &key);
    move_to(&db, "reopened", STATUS_IN_PROGRESS).unwrap();
    assert_eq!(db.with_conn(|x| escapes(x, 1, &key)).unwrap(), 1);
    let cal = db.with_conn(|x| calibration(x, &c)).unwrap();
    assert_ne!(
        cal.tiers[1].state,
        Autonomy::Withdrawn,
        "unapplied sweep verdicts must not withdraw a tier: {:?}",
        cal.tiers[1].why
    );
    // Nor are they "completed by HQ" in the day's figures: three never completed and the fourth
    // was reopened.
    let health = db.with_conn(|x| queue_health(x, &enforcing())).unwrap();
    assert_eq!(health.completed_24h, 0, "{health:?}");
}

#[test]
fn a_verdict_recorded_by_a_sweep_is_neither_a_return_nor_a_reworked_task() {
    let (db, initiative) = setup();
    let key = current_config_key(&cfg());
    waiting(&db, &initiative, "swept");
    // Two verdicts about one submission (a shadow run and a sweep) are one rejection, and the
    // task, still in review, was not sent back.
    recorded_in_review(&db, "swept", 1, VERDICT_FAIL, &key);
    recorded_in_review(&db, "swept", 1, VERDICT_FAIL, &key);
    assert_eq!(db.with_conn(|c| rejections(c, "swept")).unwrap(), 1);
    let b = db
        .with_conn(|c| briefing(c, &enforcing(), "2000-01-01 00:00:00"))
        .unwrap();
    assert_eq!(b.reworked_total, 0, "{:?}", b.reworked);
}

fn state_of(db: &Database, id: &str) -> WaitingState {
    let task = get_one(db, id);
    db.with_conn(|c| waiting_state(c, &task))
        .unwrap()
        .unwrap_or_else(|| panic!("{id} waits on no one"))
}

#[test]
fn a_task_a_sweep_recorded_a_verdict_for_but_never_moved_waits_on_the_owner_not_on_no_one() {
    let (db, initiative) = setup();
    enforce_policy(&db, &enforcing());
    let key = current_config_key(&enforcing());
    waiting(&db, &initiative, "swept-pass");
    recorded_in_review(&db, "swept-pass", 1, VERDICT_PASS, &key);
    waiting(&db, &initiative, "swept-fail");
    recorded_in_review(&db, "swept-fail", 1, VERDICT_FAIL, &key);
    for id in ["swept-pass", "swept-fail"] {
        let state = state_of(&db, id);
        assert_eq!(
            (state.who.as_str(), state.kind),
            (WAIT_OWNER, "unapplied"),
            "{id}"
        );
        assert!(state.reason.contains("sweep"), "{}", state.reason);
    }
    // A task the owner kept out of automatic verification is theirs as well, not "queued".
    waiting(&db, &initiative, "kept-out");
    db.with_conn(|c| skip_verification(c, "kept-out", "a sweep was undone"))
        .unwrap();
    let state = state_of(&db, "kept-out");
    assert_eq!((state.who.as_str(), state.kind), (WAIT_OWNER, "skipped"));
    // A task nobody has looked at yet is still the verifier's.
    waiting(&db, &initiative, "fresh");
    assert_eq!(state_of(&db, "fresh").who, WAIT_VERIFIER);
}

#[test]
fn the_queue_says_when_no_reviewer_budget_is_set_and_when_nothing_has_been_verified_for_a_day() {
    let (db, initiative) = setup();
    let cfg = enforcing();
    enforce_policy(&db, &cfg);
    waiting(&db, &initiative, "tk-1");
    db.with_conn(|c| record_wait(c, "tk-1", WAIT_BUDGET, "no reviewer is configured", 1800, 0))
        .unwrap();
    let health = db.with_conn(|c| queue_health(c, &cfg)).unwrap();
    assert!(
        health
            .alerts
            .iter()
            .any(|a| a.contains("no reviewer budget is set")),
        "{:?}",
        health.alerts
    );

    // A task queued for hours with no verdict anywhere in a day: the verifier is not running.
    waiting(&db, &initiative, "tk-2");
    assert!(
        !db.with_conn(|c| queue_health(c, &cfg))
            .unwrap()
            .alerts
            .iter()
            .any(|a| a.contains("none was verified")),
        "a task queued a moment ago is not a stall"
    );
    db.with_conn(|c| {
        // Enforcement began a day ago, so a task queued five hours ago is live work, not legacy.
        c.execute(
            "UPDATE task_verification_policy SET enforcing_since = datetime('now', '-1 day')",
            [],
        )?;
        c.execute(
            "UPDATE task_events SET occurred_at = datetime('now', '-5 hours') \
             WHERE task_id = 'tk-2' AND event_type = 'entered_ready_for_review'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let alerts = db.with_conn(|c| queue_health(c, &cfg)).unwrap().alerts;
    assert!(
        alerts
            .iter()
            .any(|a| a.contains("none was verified in the last day")),
        "{alerts:?}"
    );
}

#[test]
fn a_veto_window_for_an_earlier_submission_is_not_offered_but_is_still_cleared() {
    let (db, initiative) = setup();
    let key = current_config_key(&cfg());
    waiting(&db, &initiative, "tk-1");
    let v = recorded_in_review(&db, "tk-1", 2, VERDICT_PASS, &key);
    db.with_conn(|c| {
        create_hold(
            c,
            "tk-1",
            &v.id,
            chrono::Utc::now().timestamp() + 3600,
            &json!({}),
        )
    })
    .unwrap();
    assert!(db.with_conn(|c| get_hold(c, "tk-1")).unwrap().is_some());
    assert_eq!(db.with_conn(list_holds).unwrap().len(), 1);

    // The worker takes it back and submits again during the window.
    move_to(&db, "tk-1", STATUS_IN_PROGRESS).unwrap();
    move_to(&db, "tk-1", STATUS_READY_FOR_REVIEW).unwrap();
    assert!(db.with_conn(|c| get_hold(c, "tk-1")).unwrap().is_none());
    assert!(db.with_conn(list_holds).unwrap().is_empty());
    let err = db
        .with_conn(|c| veto_hold(c, "tk-1", "too late", "owner"))
        .unwrap_err();
    assert!(err.to_string().contains("no veto window"), "{err}");
    assert_eq!(
        db.with_conn(|c| due_holds(c, i64::MAX)).unwrap().len(),
        1,
        "the release pass still sees it, so it can clear it"
    );
}

#[test]
fn a_canary_counts_a_missed_defect_and_a_rejected_control_but_not_an_error() {
    let (db, _) = setup();
    db.with_conn(|c| {
        record_canary(c, "unmet_criterion", "fail", "fail", "m", "")?;
        record_canary(c, "unmet_criterion", "fail", "unclear", "m", "")?;
        record_canary(c, "unmet_criterion", "fail", "pass", "m", "")?;
        record_canary(c, "clear_pass", "pass", "pass", "m", "")?;
        record_canary(c, "clear_pass", "pass", "unclear", "m", "")?;
        record_canary(c, "clear_pass", "pass", "error", "m", "timed out")?;
        Ok(())
    })
    .unwrap();
    let s = db.with_conn(|c| canary_stats(c, "m")).unwrap();
    assert_eq!((s.defects, s.caught), (3, 2));
    assert_eq!((s.controls, s.passed), (2, 1));
    assert_eq!(s.false_reject_rate, Some(0.5));
    assert!(
        db.with_conn(|c| canary_stats(c, "other"))
            .unwrap()
            .catch_rate
            .is_none()
    );
}

#[test]
fn a_second_opinion_that_disagrees_is_an_audit_datum_against_the_first_verdict() {
    let (db, initiative) = setup();
    let v = verified(&db, &initiative, "tk-1", 1, "m");
    let a = db
        .with_conn(|c| {
            record_second_opinion_disagreement(c, &v.id, "the second verifier found no test")
        })
        .unwrap();
    assert_eq!(
        (
            a.source.as_str(),
            a.answer.as_deref(),
            a.verifier_config.as_str()
        ),
        (AUDIT_SECOND_OPINION, Some(AUDIT_DISAGREE), "m")
    );
    let tiers = db.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM task_audits WHERE answer = 'disagree' AND tier = 1 AND verifier_config = 'm'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(Into::into)
    });
    assert_eq!(tiers.unwrap(), 1);
}

#[test]
fn a_few_clean_audits_are_neither_trust_nor_distrust() {
    let (db, initiative) = setup();
    let c = cfg();
    let key = current_config_key(&c);
    for n in 0..4 {
        verified(&db, &initiative, &format!("tk-{n}"), 1, &key);
    }
    let audits = db
        .with_conn(|x| select_audits(x, 4, AUDIT_PULL, 1))
        .unwrap();
    for a in &audits {
        answered(&db, &a.id, true);
    }
    run_canaries(&db, &key, (4, 4), 2);
    let cal = db.with_conn(|x| calibration(x, &c)).unwrap();
    assert_eq!(cal.tiers[1].state, Autonomy::Insufficient);
    assert!(
        cal.tiers[1].why[0].contains("upper bound"),
        "{:?}",
        cal.tiers[1].why
    );
}

// ---- queue health and the briefing ----

fn enforce_policy(db: &Database, cfg: &VerificationConfig) {
    let cfg = VerificationConfig {
        mode: hq_core::config::VerificationMode::Enforce,
        ..cfg.clone()
    };
    db.with_conn(|c| sync_verification_policy(c, &cfg)).unwrap();
}

fn enforcing() -> VerificationConfig {
    VerificationConfig {
        mode: hq_core::config::VerificationMode::Enforce,
        owner_queue_alert: 5,
        owner_queue_age_days: 3,
        ..Default::default()
    }
}

/// A task in review, submitted by a worker, with nothing else done to it.
fn waiting(db: &Database, initiative: &str, id: &str) {
    make(db, id, initiative, None).unwrap();
    move_to(db, id, STATUS_IN_PROGRESS).unwrap();
    move_to(db, id, STATUS_READY_FOR_REVIEW).unwrap();
}

#[test]
fn a_weekend_of_completions_is_one_line_per_initiative_and_never_a_pass_by_pass_list() {
    let (db, initiative) = setup();
    let cfg = enforcing();
    enforce_policy(&db, &cfg);
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-2', 'personal', 'Website', 'website', 'WEB')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    for n in 0..200 {
        let target = if n % 4 == 0 {
            "in-2"
        } else {
            initiative.as_str()
        };
        verified(
            &db,
            target,
            &format!("tk-{n}"),
            u8::from(n % 50 == 0) * 2,
            "m",
        );
    }
    let b = db
        .with_conn(|c| briefing(c, &cfg, "2000-01-01 00:00:00"))
        .unwrap();
    assert_eq!(b.completed_total, 200);
    assert_eq!(b.completed.len(), 2, "grouped by initiative, not listed");
    assert_eq!(b.completed[0].count, 150);
    assert_eq!(b.completed[1].initiative, "Website");
    assert_eq!(b.completed[0].by_level.get("machine_peer"), Some(&150));
    assert_eq!(b.completed.iter().map(|g| g.tier2).sum::<i64>(), 4);
    assert!(b.needs_you.is_empty() && b.needs_you_total == 0);
    let text = briefing_text(&b);
    assert!(text.contains("Nothing needs you."), "{text}");
    assert!(text.contains("HQ verified and completed 200"), "{text}");
    assert!(!text.contains("tk-"), "no task is named for a pass: {text}");
    assert_eq!(b.audit_pool, 200, "all of it is auditable");
}

#[test]
fn what_waits_on_each_party_sums_to_the_whole_backlog() {
    let (db, initiative) = setup();
    let cfg = enforcing();
    enforce_policy(&db, &cfg);
    // queued behind the verifier, checks running, a failed task back with its worker, and a pass
    // held for the owner by the autonomy gate.
    waiting(&db, &initiative, "queued");
    waiting(&db, &initiative, "ci");
    db.with_conn(|c| record_wait(c, "ci", WAIT_CI, "checks are running", 300, 0))
        .unwrap();
    waiting(&db, &initiative, "failed");
    let obs = json!([{"id": "o1", "source": "github", "fact": "a check failed", "ok": false}]);
    db.with_conn(|c| {
        record_verdict(
            c,
            &NewVerdict {
                task_id: "failed",
                artifact: "pr:example/repo#1",
                contract_hash: "h",
                tier: 1,
                verdict: VERDICT_FAIL,
                level: LEVEL_NONE,
                verifier: "verify:a",
                independence: "none",
                observations: &obs,
                criteria: &json!([]),
                reasons: "a required check failed",
                applied: false,
                canary: false,
                submission: None,
                waiting_for: Some(WAIT_WORKER),
                verifier_config: "m",
            },
        )?;
        send_back(c, "failed", "verify:a", "a required check failed", 3)
    })
    .unwrap();
    waiting(&db, &initiative, "held");
    db.with_conn(|c| {
        record_verdict(
            c,
            &NewVerdict {
                task_id: "held",
                artifact: "pr:example/repo#2",
                contract_hash: "h",
                tier: 1,
                verdict: VERDICT_PASS,
                level: LEVEL_MACHINE_PEER,
                verifier: "verify:b",
                independence: "different_session",
                observations: &json!([{"id": "o1", "source": "github", "fact": "ok"}]),
                criteria: &json!([{"text": "x", "result": "pass", "observation_ids": ["o1"]}]),
                reasons: "Held for the owner: the reviewer missed planted defects.",
                applied: false,
                canary: false,
                submission: review_marker(c, "held")?,
                waiting_for: Some(WAIT_OWNER),
                verifier_config: "m",
            },
        )
        .map(|_| ())
    })
    .unwrap();

    let health = db.with_conn(|c| queue_health(c, &cfg)).unwrap();
    assert_eq!(health.backlog.get("verifier"), Some(&1));
    assert_eq!(health.backlog.get("ci"), Some(&1));
    assert_eq!(health.backlog.get("worker"), Some(&1));
    assert_eq!(health.backlog.get("owner"), Some(&1));
    assert_eq!(
        health.backlog.values().sum::<i64>(),
        4,
        "every waiting task is counted once"
    );
    assert_eq!(health.owner_by_kind.get("held"), Some(&1));
    assert_eq!(health.deferrals[0].waiting_for, "ci");
    assert!(health.alerts.is_empty(), "{:?}", health.alerts);
    let b = db
        .with_conn(|c| briefing(c, &cfg, "2000-01-01 00:00:00"))
        .unwrap();
    assert_eq!(b.needs_you.len(), 1);
    assert!(!b.needs_you[0].display_id.is_empty());
    assert_eq!(b.reworked_total, 1);
    assert!(
        b.reworked[0].last_finding.contains("required check"),
        "{:?}",
        b.reworked
    );
}

#[test]
fn past_the_threshold_the_list_becomes_one_offer_and_an_old_item_raises_one_alert() {
    let (db, initiative) = setup();
    let cfg = enforcing();
    // Work from before enforcement: a sweep's job.
    for n in 0..8 {
        waiting(&db, &initiative, &format!("old-{n}"));
    }
    enforce_policy(&db, &cfg);
    db.with_conn(|c| {
        c.execute(
            "UPDATE task_verification_policy SET enforcing_since = datetime('now', '+1 hour')",
            [],
        )?;
        c.execute(
            "UPDATE task_events SET occurred_at = datetime('now', '-5 days') \
             WHERE event_type = 'entered_ready_for_review'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let b = db
        .with_conn(|c| briefing(c, &cfg, "2000-01-01 00:00:00"))
        .unwrap();
    assert_eq!(b.needs_you_total, 8);
    assert!(b.needs_you.is_empty(), "no flat list past the threshold");
    let offer = b.bulk_offer.expect("an offer");
    assert_eq!(offer.count, 8);
    assert!(
        offer.text.contains("before verification was enforced"),
        "{}",
        offer.text
    );
    assert_eq!(offer.command, "hq verify sweep plan");
    assert_eq!(b.health.owner_by_kind.get("legacy"), Some(&8));
    let alerts = b.health.alerts.join(" | ");
    assert!(alerts.contains("8 items wait on you"), "{alerts}");
    assert!(alerts.contains("5 days old"), "{alerts}");
}

#[test]
fn spending_the_reviewer_budget_is_an_alert_only_while_tasks_actually_wait_for_it() {
    let (db, initiative) = setup();
    let cfg = VerificationConfig {
        reviewer_monthly_ceiling_usd: Some(1.0),
        ..enforcing()
    };
    enforce_policy(&db, &cfg);
    waiting(&db, &initiative, "tk-1");
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO task_outcomes (session_id, turn_idx, model, provider, task_hint, latency_ms, \
                 cost_usd, success, recorded_at, origin) \
             VALUES ('s', 0, 'm', 'p', 'h', 1, 5.0, 1, strftime('%s','now'), 'verification')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(
        db.with_conn(|c| queue_health(c, &cfg))
            .unwrap()
            .alerts
            .is_empty(),
        "spent, but nothing waits for it"
    );
    db.with_conn(|c| {
        record_wait(
            c,
            "tk-1",
            WAIT_BUDGET,
            "the monthly reviewer ceiling is used up",
            1800,
            0,
        )
    })
    .unwrap();
    let health = db.with_conn(|c| queue_health(c, &cfg)).unwrap();
    assert!(
        health
            .alerts
            .iter()
            .any(|a| a.contains("budget") && a.contains("machine checks continue")),
        "{:?}",
        health.alerts
    );
    assert_eq!(health.spent_this_month_usd, 5.0);
}

#[test]
fn with_verification_off_there_is_nothing_to_alert_about() {
    let (db, initiative) = setup();
    for n in 0..30 {
        waiting(&db, &initiative, &format!("tk-{n}"));
    }
    let off = VerificationConfig::default();
    let b = db
        .with_conn(|c| briefing(c, &off, "2000-01-01 00:00:00"))
        .unwrap();
    assert!(b.bulk_offer.is_none() && b.health.alerts.is_empty());
}
