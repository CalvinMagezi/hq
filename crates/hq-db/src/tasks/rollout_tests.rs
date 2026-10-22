use super::calibration_tests::{answered, verified};
use super::tests::{make, move_to, setup};
use super::*;
use crate::Database;
use hq_core::config::{VerificationConfig, VerificationMode};
use serde_json::json;

const OWNER: &str = "web";
const WORKER: &str = "builder";

fn move_as(db: &Database, id: &str, status: &str, actor: &str) {
    db.with_conn(|c| {
        update_task_as(
            c,
            id,
            &TaskPatch {
                status: Some(status.into()),
                ..Default::default()
            },
            None,
            &WriteCtx {
                actor: Some(actor),
                ..Default::default()
            },
        )
    })
    .unwrap();
}

/// A task a worker has put in review; returns the id of that submission.
fn submitted(db: &Database, initiative: &str, id: &str) -> i64 {
    make(db, id, initiative, None).unwrap();
    resubmit(db, id)
}

fn resubmit(db: &Database, id: &str) -> i64 {
    move_as(db, id, STATUS_IN_PROGRESS, WORKER);
    move_as(db, id, STATUS_READY_FOR_REVIEW, WORKER);
    db.with_conn(|c| review_marker(c, id)).unwrap().unwrap()
}

/// A verdict that moved nothing, as shadow mode records them.
fn shadow_verdict(db: &Database, id: &str, tier: u8, verdict: &str, submission: i64) {
    verdict_with(db, id, tier, verdict, submission, false, false);
}

fn verdict_with(
    db: &Database,
    id: &str,
    tier: u8,
    verdict: &str,
    submission: i64,
    applied: bool,
    canary: bool,
) {
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
                level: LEVEL_MACHINE_PEER,
                verifier: "verify:run-1",
                independence: "different_session",
                observations: &obs,
                criteria: &crit,
                reasons: "",
                applied,
                canary,
                submission: Some(submission),
                waiting_for: None,
                verifier_config: "m",
            },
        )
    })
    .unwrap();
}

fn compare(db: &Database) -> Vec<ShadowTier> {
    db.with_conn(shadow_comparison).unwrap()
}

#[test]
fn shadow_verdicts_are_compared_with_what_someone_else_did_next() {
    let (db, initiative) = setup();
    // Tier 0: one agreement, one false pass (the dangerous kind), one false reject.
    let a = submitted(&db, &initiative, "agree");
    shadow_verdict(&db, "agree", 0, VERDICT_PASS, a);
    move_as(&db, "agree", STATUS_COMPLETE, OWNER);
    let b = submitted(&db, &initiative, "false-pass");
    shadow_verdict(&db, "false-pass", 0, VERDICT_PASS, b);
    move_as(&db, "false-pass", STATUS_TO_DO, OWNER);
    let c = submitted(&db, &initiative, "false-reject");
    shadow_verdict(&db, "false-reject", 0, VERDICT_FAIL, c);
    move_as(&db, "false-reject", STATUS_COMPLETE, OWNER);
    // Tier 1: a fail that was right, whatever status the work was sent back to.
    for (n, back) in [STATUS_TO_DO, STATUS_IN_PROGRESS, STATUS_BLOCKED]
        .into_iter()
        .enumerate()
    {
        let id = format!("sent-back-{n}");
        let s = submitted(&db, &initiative, &id);
        shadow_verdict(&db, &id, 1, VERDICT_FAIL, s);
        move_as(&db, &id, back, OWNER);
    }

    let tiers = compare(&db);
    assert_eq!(tiers.len(), 3);
    let t0 = &tiers[0];
    assert_eq!(
        (t0.compared, t0.agreed, t0.false_passes, t0.false_rejects),
        (3, 1, 1, 1)
    );
    assert!(
        t0.false_pass_upper.unwrap() > 0.5,
        "one in three is not rare"
    );
    let t1 = &tiers[1];
    assert_eq!(
        (t1.compared, t1.agreed, t1.false_passes, t1.false_rejects),
        (3, 3, 0, 0)
    );
    assert!(t1.false_pass_upper.unwrap() < 0.5);
    assert_eq!(tiers[2].compared, 0);
    assert!(
        tiers[2].false_pass_upper.is_none(),
        "no rate is invented from nothing"
    );
}

#[test]
fn a_move_by_the_worker_the_verifier_or_nobody_known_says_nothing() {
    let (db, initiative) = setup();
    // The worker completing its own work.
    let a = submitted(&db, &initiative, "own");
    shadow_verdict(&db, "own", 0, VERDICT_FAIL, a);
    move_as(&db, "own", STATUS_COMPLETE, WORKER);
    // HQ's own verifier or sweep.
    let b = submitted(&db, &initiative, "by-hq");
    shadow_verdict(&db, "by-hq", 0, VERDICT_PASS, b);
    move_as(&db, "by-hq", STATUS_TO_DO, "verify:sweep-sw-1");
    // A move nobody is recorded as having made.
    let c = submitted(&db, &initiative, "anonymous");
    shadow_verdict(&db, "anonymous", 0, VERDICT_PASS, c);
    move_to(&db, "anonymous", STATUS_COMPLETE).unwrap();
    // Still in review: nothing has happened yet.
    let d = submitted(&db, &initiative, "waiting");
    shadow_verdict(&db, "waiting", 0, VERDICT_PASS, d);
    // The one that counts: the owner decided it.
    let e = submitted(&db, &initiative, "decided");
    shadow_verdict(&db, "decided", 0, VERDICT_PASS, e);
    move_as(&db, "decided", STATUS_COMPLETE, OWNER);

    let t0 = &compare(&db)[0];
    assert_eq!(
        t0.compared, 1,
        "only the last one was decided by someone independent"
    );
    assert_eq!(t0.agreed, 1);
}

#[test]
fn only_the_newest_verdict_counts_and_neither_applied_nor_canary_ones_do() {
    let (db, initiative) = setup();
    let s = submitted(&db, &initiative, "tk-1");
    shadow_verdict(&db, "tk-1", 0, VERDICT_FAIL, s);
    shadow_verdict(&db, "tk-1", 0, VERDICT_PASS, s);
    // HQ's own applied verdict and a planted canary are measured by audits and canary runs.
    verdict_with(&db, "tk-1", 0, VERDICT_PASS, s, true, false);
    verdict_with(&db, "tk-1", 0, VERDICT_FAIL, s, false, true);
    move_as(&db, "tk-1", STATUS_COMPLETE, OWNER);

    let t0 = &compare(&db)[0];
    assert_eq!((t0.compared, t0.agreed), (1, 1), "{t0:?}");
}

#[test]
fn a_resubmission_is_compared_on_its_own_verdict_and_outcome() {
    let (db, initiative) = setup();
    let first = submitted(&db, &initiative, "tk-1");
    shadow_verdict(&db, "tk-1", 1, VERDICT_FAIL, first);
    // Sent back into progress by the owner: a decision, which does not make them a worker.
    move_as(&db, "tk-1", STATUS_IN_PROGRESS, OWNER);
    let second = resubmit(&db, "tk-1");
    assert!(second > first);
    shadow_verdict(&db, "tk-1", 1, VERDICT_PASS, second);
    move_as(&db, "tk-1", STATUS_COMPLETE, OWNER);

    let t1 = &compare(&db)[1];
    assert_eq!(
        (t1.compared, t1.agreed, t1.false_passes, t1.false_rejects),
        (2, 2, 0, 0)
    );
}

fn cfg() -> VerificationConfig {
    VerificationConfig {
        autonomy_min_audits: 4,
        autonomy_min_canaries: 4,
        autonomy_max_disagreement: 0.5,
        ..Default::default()
    }
}

fn shadow_cfg() -> VerificationConfig {
    VerificationConfig {
        mode: VerificationMode::Shadow,
        ..cfg()
    }
}

fn enforce_cfg(tiers: &[u8]) -> VerificationConfig {
    VerificationConfig {
        mode: VerificationMode::Enforce,
        enforce_tiers: tiers.to_vec(),
        ..cfg()
    }
}

fn step(db: &Database, c: &VerificationConfig) -> RolloutReport {
    db.with_conn(|x| rollout(x, c)).unwrap()
}

fn gate<'a>(r: &'a RolloutReport, label_part: &str) -> &'a Gate {
    r.gates
        .iter()
        .find(|g| g.label.contains(label_part))
        .unwrap_or_else(|| panic!("no gate with {label_part:?} in {:?}", r.gates))
}

/// `n` tier-0 shadow verdicts that an independent party then agreed with.
fn agreed_in_shadow(db: &Database, initiative: &str, n: usize) {
    for i in 0..n {
        let id = format!("shadow-{i}");
        let s = submitted(db, initiative, &id);
        shadow_verdict(db, &id, 0, VERDICT_PASS, s);
        move_as(db, &id, STATUS_COMPLETE, OWNER);
    }
}

fn with_reviewer(c: VerificationConfig) -> VerificationConfig {
    VerificationConfig {
        reviewer_monthly_ceiling_usd: Some(5.0),
        ..c
    }
}

fn catches_planted_defects(db: &Database, c: &VerificationConfig) {
    let key = current_config_key(c);
    db.with_conn(|x| {
        for _ in 0..4 {
            record_canary(x, "unmet_criterion", "fail", "fail", &key, "")?;
        }
        record_canary(x, "clear_pass", "pass", "pass", &key, "")?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn an_instance_that_is_off_is_offered_shadow_and_nothing_else() {
    let (db, _) = setup();
    let r = step(&db, &VerificationConfig::default());
    assert_eq!(r.stage, "off");
    let next = r.next.expect("a next step");
    assert!(next.change.contains("mode: shadow"), "{next:?}");
    assert!(r.gates.is_empty() && r.ready && r.warnings.is_empty());
}

#[test]
fn shadow_waits_on_numbers_a_reviewer_and_the_backlog_and_says_which() {
    let (db, initiative) = setup();
    waiting(&db, &initiative, "backlog-1");
    let r = step(&db, &shadow_cfg());
    assert_eq!(r.stage, "shadow");
    assert!(!r.ready);
    assert_eq!(
        r.next.as_ref().unwrap().label,
        "Enforce tier 0 (notes, research, planning)"
    );
    assert!(
        r.next
            .as_ref()
            .unwrap()
            .change
            .contains("enforce_tiers: [0]")
    );
    let seen = gate(&r, "Enough tier 0 verdicts");
    assert!(!seen.met);
    assert!(
        seen.detail.starts_with("0 of 4 compared"),
        "{}",
        seen.detail
    );
    assert_eq!(gate(&r, "false passes").detail, "nothing compared yet");
    assert!(!gate(&r, "reviewer is configured").met);
    let backlog = gate(&r, "backlog");
    assert!(!backlog.met);
    assert!(
        backlog.detail.contains("hq verify sweep plan"),
        "{}",
        backlog.detail
    );
}

/// A task in review, submitted by a worker, with nothing else done to it.
fn waiting(db: &Database, initiative: &str, id: &str) {
    submitted(db, initiative, id);
}

#[test]
fn every_condition_met_opens_the_step() {
    let (db, initiative) = setup();
    let c = with_reviewer(shadow_cfg());
    agreed_in_shadow(&db, &initiative, 4);
    catches_planted_defects(&db, &c);
    let r = step(&db, &c);
    assert!(r.ready, "{:#?}", r.gates);
    assert!(r.gates.iter().all(|g| g.met));
    assert!(gate(&r, "false passes").detail.contains("limit is 50%"));
}

#[test]
fn a_false_pass_in_shadow_closes_the_gate_on_that_tier() {
    let (db, initiative) = setup();
    let c = with_reviewer(shadow_cfg());
    agreed_in_shadow(&db, &initiative, 3);
    let s = submitted(&db, &initiative, "bad");
    shadow_verdict(&db, "bad", 0, VERDICT_PASS, s);
    move_as(&db, "bad", STATUS_TO_DO, OWNER);
    catches_planted_defects(&db, &c);
    let r = step(&db, &c);
    assert!(gate(&r, "Enough tier 0 verdicts").met);
    assert!(
        !gate(&r, "false passes").met,
        "one in four is not rare enough"
    );
    assert!(!r.ready);
}

#[test]
fn a_reviewer_that_misses_planted_defects_does_not_open_the_step() {
    let (db, initiative) = setup();
    let c = with_reviewer(shadow_cfg());
    agreed_in_shadow(&db, &initiative, 4);
    let key = current_config_key(&c);
    db.with_conn(|x| {
        for got in ["fail", "pass", "pass", "pass"] {
            record_canary(x, "unmet_criterion", "fail", got, &key, "")?;
        }
        Ok(())
    })
    .unwrap();
    let r = step(&db, &c);
    let g = gate(&r, "catches planted defects");
    assert!(!g.met);
    assert!(g.detail.contains("caught 1 of 4"), "{}", g.detail);
}

#[test]
fn a_sweep_that_planned_the_backlog_meets_its_gate_and_a_cancelled_one_does_not() {
    let (db, initiative) = setup();
    waiting(&db, &initiative, "backlog-1");
    let c = shadow_cfg();
    assert!(!gate(&step(&db, &c), "backlog").met);
    let sweep = db
        .with_conn(|x| {
            create_sweep(
                x,
                &SweepScope::All,
                &[NewSweepItem {
                    task_id: "backlog-1".into(),
                    rank: 0,
                    submission: None,
                    status_before: STATUS_READY_FOR_REVIEW.into(),
                    tier: 0,
                }],
            )
        })
        .unwrap();
    assert!(gate(&step(&db, &c), "backlog").met);
    db.with_conn(|x| set_sweep_status(x, &sweep.id, &[SWEEP_PLANNING], SWEEP_CANCELLED))
        .unwrap();
    assert!(!gate(&step(&db, &c), "backlog").met);
}

#[test]
fn tier_one_waits_for_tier_zero_to_earn_its_autonomy() {
    let (db, initiative) = setup();
    let c = with_reviewer(enforce_cfg(&[0]));
    let r = step(&db, &c);
    assert_eq!(r.stage, "tier_0");
    assert!(r.next.as_ref().unwrap().change.contains("[0, 1]"));
    assert!(
        gate(&r, "Enough tier 1 verdicts")
            .detail
            .starts_with("0 of 4")
    );
    assert!(!gate(&r, "Tier 0 has earned").met);

    // Four audited, clean tier 0 completions, and a reviewer that catches what is planted.
    let key = current_config_key(&c);
    for n in 0..4 {
        verified(&db, &initiative, &format!("done-{n}"), 0, &key);
    }
    let audits = db
        .with_conn(|x| select_audits(x, 4, AUDIT_PULL, 1))
        .unwrap();
    for a in &audits {
        answered(&db, &a.id, true);
    }
    catches_planted_defects(&db, &c);
    assert!(gate(&step(&db, &c), "Tier 0 has earned").met);
}

#[test]
fn an_enforced_tier_whose_calibration_was_withdrawn_is_a_warning_at_any_stage() {
    let (db, initiative) = setup();
    let c = with_reviewer(enforce_cfg(&[0, 1]));
    let key = current_config_key(&c);
    for n in 0..4 {
        verified(&db, &initiative, &format!("done-{n}"), 1, &key);
    }
    let audits = db
        .with_conn(|x| select_audits(x, 4, AUDIT_PULL, 1))
        .unwrap();
    for a in &audits {
        answered(&db, &a.id, false);
    }
    let r = step(&db, &c);
    assert_eq!(r.stage, "tier_1");
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    assert!(r.warnings[0].starts_with("tier 1 is enforced but its calibration is withdrawn"));
    // The same numbers are no warning for a tier that is not enforced.
    let shadow = step(&db, &with_reviewer(shadow_cfg()));
    assert!(shadow.warnings.is_empty());
}

#[test]
fn with_every_tier_enforced_there_is_no_next_step_and_enforcing_no_tier_reads_as_shadow() {
    let (db, _) = setup();
    let all = step(&db, &enforce_cfg(&[0, 1, 2]));
    assert_eq!(all.stage, "all_tiers");
    assert!(all.next.is_none() && all.gates.is_empty() && all.ready);
    assert_eq!(step(&db, &enforce_cfg(&[])).stage, "shadow");
}
