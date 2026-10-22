//! Turning verification on in steps, each gated on numbers and not on elapsed time
//! (docs/plans/task-verification.md, WS7). `rollout` says where an instance is, what stands
//! between it and the next step, and the one change that takes the step. It changes nothing.
//!
//! off, then shadow (verdicts that move nothing), then enforce for tier 0, then tiers 0 and 1,
//! then all three. A tier may be enforced when what shadow mode saw of it holds up (enough
//! verdicts compared with what independent people and agents then did, and a low enough rate of
//! false passes), the reviewer has been tested and passed, the tiers already enforced have earned
//! their autonomy, and the backlog has been read by a sweep.

use super::calibration::{Autonomy, calibration};
use super::shadow::{ShadowTier, shadow_comparison};
use super::sweeps::{SWEEP_CANCELLED, list_sweeps};
use super::verification::MAX_TIER;
use super::*;
use hq_core::config::{VerificationConfig, VerificationMode};

/// One condition for the next step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gate {
    pub label: String,
    pub met: bool,
    /// The numbers behind it, in a sentence.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextStep {
    pub label: String,
    /// The setting to change, as it is written in the config.
    pub change: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RolloutReport {
    /// `off`, `shadow`, `tier_0`, `tier_1` or `all_tiers`.
    pub stage: &'static str,
    pub stage_label: String,
    /// What the next step would be; `None` once every tier is enforced.
    pub next: Option<NextStep>,
    /// What stands between this instance and that step.
    pub gates: Vec<Gate>,
    /// Every gate is met.
    pub ready: bool,
    /// Things worth attention now, whatever the stage: an enforced tier whose calibration has
    /// been withdrawn, for example.
    pub warnings: Vec<String>,
    /// What shadow mode's verdicts looked like next to what independent people and agents did.
    pub shadow: Vec<ShadowTier>,
}

fn stage_of(cfg: &VerificationConfig) -> (&'static str, String, Option<u8>) {
    match cfg.mode {
        VerificationMode::Off => ("off", "Off: nothing is verified".into(), None),
        VerificationMode::Shadow => (
            "shadow",
            "Shadow: verdicts are recorded and nothing moves".into(),
            None,
        ),
        VerificationMode::Enforce => {
            let top = cfg.enforce_tiers.iter().copied().max();
            match top {
                None => (
                    "shadow",
                    "Enforcing no tier: the same as shadow".into(),
                    None,
                ),
                Some(0) => ("tier_0", "Enforcing tier 0".into(), Some(0)),
                Some(1) => ("tier_1", "Enforcing tiers up to 1".into(), Some(1)),
                Some(_) => ("all_tiers", "Enforcing every tier".into(), Some(MAX_TIER)),
            }
        }
    }
}

fn percent(rate: f64) -> String {
    format!("{:.0}%", rate * 100.0)
}

/// Where the instance is in the rollout and what the next step needs.
pub fn rollout(conn: &Connection, cfg: &VerificationConfig) -> Result<RolloutReport> {
    let (stage, stage_label, enforced_to) = stage_of(cfg);
    let cal = calibration(conn, cfg)?;
    let shadow = shadow_comparison(conn)?;

    let mut warnings = Vec::new();
    if let Some(top) = enforced_to {
        for t in cal.tiers.iter().filter(|t| t.tier <= top) {
            if t.state == Autonomy::Withdrawn {
                warnings.push(format!(
                    "tier {} is enforced but its calibration is withdrawn ({}). Narrow enforce_tiers or read hq verify status.",
                    t.tier,
                    t.why.first().cloned().unwrap_or_default()
                ));
            }
        }
    }

    // The tier the next step would start enforcing, and the setting that does it.
    let next_tier: Option<u8> = match stage {
        "off" | "shadow" => Some(0),
        "tier_0" => Some(1),
        "tier_1" => Some(2),
        _ => None,
    };
    let Some(tier) = next_tier else {
        return Ok(RolloutReport {
            stage,
            stage_label,
            next: None,
            gates: Vec::new(),
            ready: true,
            warnings,
            shadow,
        });
    };
    let next = match stage {
        "off" => NextStep {
            label: "Turn verification on in shadow mode".into(),
            change: "tasks.verification.mode: shadow".into(),
        },
        "shadow" => NextStep {
            label: "Enforce tier 0 (notes, research, planning)".into(),
            change: "tasks.verification.mode: enforce, with enforce_tiers: [0]".into(),
        },
        "tier_0" => NextStep {
            label: "Also enforce tier 1 (code with a pull request)".into(),
            change: "tasks.verification.enforce_tiers: [0, 1]".into(),
        },
        _ => NextStep {
            label: "Also enforce tier 2 (deploys, migrations, security, payments)".into(),
            change: "tasks.verification.enforce_tiers: [0, 1, 2]".into(),
        },
    };
    if stage == "off" {
        return Ok(RolloutReport {
            stage,
            stage_label,
            next: Some(next),
            gates: Vec::new(),
            ready: true,
            warnings,
            shadow,
        });
    }

    let mut gates = Vec::new();
    let mut gate = |label: &str, met: bool, detail: String| {
        gates.push(Gate {
            label: label.to_string(),
            met,
            detail,
        });
    };

    // What shadow mode saw of the tier about to be enforced.
    let s = &shadow[usize::from(tier)];
    let need = i64::from(cfg.autonomy_min_audits);
    gate(
        &format!("Enough tier {tier} verdicts compared with what happened next"),
        s.compared >= need,
        format!(
            "{} of {need} compared: {} agreed, {} would have completed work that was sent back, {} would have held back work that was accepted",
            s.compared, s.agreed, s.false_passes, s.false_rejects
        ),
    );
    let bound_ok = s
        .false_pass_upper
        .is_some_and(|u| u <= cfg.autonomy_max_disagreement);
    gate(
        &format!("Tier {tier} false passes are rare enough"),
        s.compared >= need && bound_ok,
        match s.false_pass_upper {
            Some(u) => format!(
                "at most {} of verdicts (95% confidence) would have completed work that was sent back; the limit is {}",
                percent(u),
                percent(cfg.autonomy_max_disagreement)
            ),
            None => "nothing compared yet".to_string(),
        },
    );

    // The reviewer has been tested, if a reviewer is used at all.
    if cfg.reviewer_enabled() {
        let c = &cal.canaries;
        let need_c = i64::from(cfg.autonomy_min_canaries);
        let catch_ok = c
            .catch_rate
            .is_some_and(|r| r >= cfg.autonomy_min_catch_rate);
        gate(
            "The reviewer catches planted defects",
            c.defects >= need_c && catch_ok,
            match c.catch_rate {
                Some(r) => format!(
                    "caught {} of {} planted defects ({}); it needs {need_c} tried and {}",
                    c.caught,
                    c.defects,
                    percent(r),
                    percent(cfg.autonomy_min_catch_rate)
                ),
                None => format!("no planted defects tried yet; it needs {need_c}"),
            },
        );
    } else {
        gate(
            "A reviewer is configured",
            false,
            "tasks.verification.reviewer_monthly_ceiling_usd is not set, so no model is called and no task can reach the level its tier needs".to_string(),
        );
    }

    // Tiers already enforced have earned their autonomy.
    for lower in cal.tiers.iter().filter(|t| t.tier < tier) {
        gate(
            &format!("Tier {} has earned its autonomy", lower.tier),
            lower.state == Autonomy::Earned,
            lower.why.join("; "),
        );
    }

    // The backlog has been read by a sweep, if there is one.
    let waiting: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'ready_for_review' AND archived_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let swept = list_sweeps(conn, 50)?
        .iter()
        .any(|sw| sw.status != SWEEP_CANCELLED);
    gate(
        "The backlog has been read by a sweep",
        waiting == 0 || swept,
        if waiting == 0 {
            "nothing is waiting for review".to_string()
        } else if swept {
            format!("{waiting} tasks wait for review and a sweep has planned them")
        } else {
            format!(
                "{waiting} tasks wait for review: hq verify sweep plan reads them without a model"
            )
        },
    );

    let ready = gates.iter().all(|g| g.met);
    Ok(RolloutReport {
        stage,
        stage_label,
        next: Some(next),
        gates,
        ready,
        warnings,
        shadow,
    })
}
