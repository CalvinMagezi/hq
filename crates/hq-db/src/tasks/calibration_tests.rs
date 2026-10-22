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
fn verified(db: &Database, initiative: &str, id: &str, tier: u8, config: &str) -> Verdict {
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
    move_to(db, id, STATUS_COMPLETE).unwrap();
    v
}

fn answered(db: &Database, id: &str, agrees: bool) {
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
