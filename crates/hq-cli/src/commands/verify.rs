//! `hq verify`: how far HQ's own verification of finished tasks can be trusted, and the audits the
//! owner answers to find out. Reads and writes this machine's database only. It is deliberately not
//! a tool an agent can call: an agent must not be able to agree with an audit of its own work.

use std::io::Write;

use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::tasks as t;

/// Audits drawn by one `pull` when no count is given.
const DEFAULT_PULL: usize = 10;
/// Most audits drawn by one `pull`.
const MAX_PULL: usize = 100;

/// What was typed after `hq verify`.
#[derive(Debug, Default, Clone)]
pub struct Opts {
    pub sub: String,
    /// A count (pull), an audit id (agree, disagree), a task (veto) or a sweep verb (sweep).
    pub arg: Option<String>,
    /// A sweep id for the sweep verbs.
    pub extra: Option<String>,
    pub note: Option<String>,
    pub json: bool,
    /// Sweep scope: one initiative.
    pub initiative: Option<String>,
    /// Sweep scope: entered review on or after this date (YYYY-MM-DD).
    pub since: Option<String>,
    /// Confirm an apply that moves more than `sweep_confirm_over` tasks.
    pub confirm: bool,
}

pub async fn run(config: &HqConfig, opts: Opts) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    render(
        &db,
        &config.tasks.verification,
        &opts,
        &mut std::io::stdout(),
    )
}

fn render(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    opts: &Opts,
    out: &mut dyn Write,
) -> Result<()> {
    let (sub, json) = (opts.sub.as_str(), opts.json);
    let (arg, note) = (opts.arg.clone(), opts.note.clone());
    match sub {
        "status" | "" => status(db, cfg, json, out),
        "rollout" => rollout(db, cfg, json, out),
        "audits" => audits(db, json, out),
        "holds" => holds(db, json, out),
        "queue" => queue(db, cfg, json, out),
        "briefing" => {
            let hours = match &arg {
                Some(h) => h
                    .trim_end_matches('h')
                    .parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("briefing takes a number of hours, not '{h}'"))?,
                None => 24,
            }
            .clamp(1, 24 * 31);
            let since = (chrono::Utc::now() - chrono::Duration::hours(hours))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let b = db.with_conn(|c| t::briefing(c, cfg, &since))?;
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(&b)?)?;
                return Ok(());
            }
            print_briefing(&b, hours, out)
        }
        "sweep" | "sweeps" => sweeps(db, cfg, opts, out),
        "veto" => {
            let Some(task) = arg else {
                bail!("usage: hq verify veto <task> [--note <why>]");
            };
            db.with_conn(|c| t::veto_hold(c, &task, note.as_deref().unwrap_or_default(), "owner"))?;
            writeln!(
                out,
                "Vetoed. {task} is back in to_do with your note on its thread."
            )?;
            Ok(())
        }
        "pull" => {
            let count = match arg {
                Some(n) => n
                    .parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("pull takes a number of audits, not '{n}'"))?,
                None => DEFAULT_PULL,
            }
            .clamp(1, MAX_PULL);
            let seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(1) as u64;
            let picked = db.with_conn(|c| t::select_audits(c, count, t::AUDIT_PULL, seed))?;
            if picked.is_empty() {
                writeln!(
                    out,
                    "Nothing to audit: no task HQ completed is waiting for an audit."
                )?;
                return Ok(());
            }
            writeln!(out, "{} audits drawn.\n", picked.len())?;
            audits(db, json, out)
        }
        "agree" | "disagree" => {
            let Some(id) = arg else {
                bail!("usage: hq verify {sub} <audit-id> [--note <why>]");
            };
            let answered = db.with_conn(|c| {
                t::answer_audit(
                    c,
                    &id,
                    sub == "agree",
                    note.as_deref().unwrap_or_default(),
                    "owner",
                )
            })?;
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(&answered)?)?;
            } else if sub == "agree" {
                writeln!(out, "Recorded: you agree with HQ's verdict.")?;
            } else {
                writeln!(
                    out,
                    "Recorded: you disagree. The task is back in to_do with your note on its thread."
                )?;
            }
            Ok(())
        }
        _ => {
            writeln!(
                out,
                "Usage: hq verify [status|audits|pull [n]|agree <id>|disagree <id>|holds|veto <task>|sweep ...]\n"
            )?;
            for (verb, what) in HELP {
                writeln!(out, "  {verb:<12} {what}")?;
            }
            Ok(())
        }
    }
}

const HELP: [(&str, &str); 11] = [
    (
        "status",
        "Mode, who tasks are waiting on, and whether the verifier has earned its autonomy",
    ),
    (
        "rollout",
        "Where turning verification on stands, what the next step waits for, and the one setting that takes it",
    ),
    ("audits", "Completed tasks waiting for your agree or not"),
    (
        "pull [n]",
        "Draw more audits now, favouring what has the least data (default 10)",
    ),
    ("agree", "You agree HQ was right to complete the task"),
    (
        "disagree",
        "You do not: the task is reopened and counts against the verifier (--note says why)",
    ),
    (
        "holds",
        "Tier 2 tasks that passed and complete when their veto window closes",
    ),
    (
        "veto",
        "Reverse one of those inside its window (--note says why)",
    ),
    (
        "briefing",
        "What needs you, then what HQ verified and reworked while you were away (hours, default 24)",
    ),
    (
        "queue",
        "Who tasks are waiting on, how old, how fast it clears, and spend against the ceiling",
    ),
    (
        "sweep",
        "Clear a queue in one go: plan [--initiative X] [--since DATE], list, show|run|pause|cancel|apply|undo <id> (apply --confirm)",
    ),
];

fn rollout(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let r = db.with_conn(|c| t::rollout(c, cfg))?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&r)?)?;
        return Ok(());
    }
    writeln!(out, "Rollout: {}\n", r.stage_label)?;
    for w in &r.warnings {
        writeln!(out, "Warning: {w}")?;
    }
    if !r.warnings.is_empty() {
        writeln!(out)?;
    }
    if r.shadow.iter().any(|s| s.compared > 0) {
        writeln!(
            out,
            "Shadow verdicts next to what someone else then did with the work:"
        )?;
        for s in r.shadow.iter().filter(|s| s.compared > 0) {
            writeln!(
                out,
                "  Tier {}: {} compared, {} agreed, {} would have completed work that was sent back, {} would have held back work that was accepted{}",
                s.tier,
                s.compared,
                s.agreed,
                s.false_passes,
                s.false_rejects,
                s.false_pass_upper
                    .map(|u| format!(" (false passes at most {:.0}%)", u * 100.0))
                    .unwrap_or_default()
            )?;
        }
        writeln!(out)?;
    }
    let Some(next) = &r.next else {
        writeln!(out, "Every tier is enforced. Nothing is left to turn on.")?;
        return Ok(());
    };
    writeln!(out, "Next: {}", next.label)?;
    for g in &r.gates {
        let mark = if g.met { "[x]" } else { "[ ]" };
        writeln!(out, "  {mark} {}\n        {}", g.label, g.detail)?;
    }
    let open = r.gates.iter().filter(|g| !g.met).count();
    if open == 0 {
        writeln!(
            out,
            "\nEvery condition is met. The change is: {}",
            next.change
        )?;
    } else {
        writeln!(
            out,
            "\n{open} {} not met yet. The change, when they are: {}",
            if open == 1 {
                "condition is"
            } else {
                "conditions are"
            },
            next.change
        )?;
    }
    Ok(())
}

fn percent(rate: Option<f64>) -> String {
    rate.map_or_else(|| "no data".to_string(), |r| format!("{:.0}%", r * 100.0))
}

fn status(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let cal = db.with_conn(|c| t::calibration(c, cfg))?;
    let waiting = db.with_conn(t::waiting_summary)?;
    if json {
        let value = serde_json::json!({
            "mode": cfg.mode,
            "autonomy_gate": cfg.autonomy_gate,
            "waiting_on": waiting,
            "calibration": cal,
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    writeln!(
        out,
        "Verification: {:?}, autonomy gate {:?}, verifier {}\n",
        cfg.mode, cfg.autonomy_gate, cal.verifier_config
    )?;
    if waiting.is_empty() {
        writeln!(out, "No task is waiting on anyone.")?;
    } else {
        writeln!(out, "Waiting on:")?;
        for (who, n) in &waiting {
            let mark = if who == t::WAIT_OWNER { "  <- you" } else { "" };
            writeln!(out, "  {who:<9} {n}{mark}")?;
        }
    }
    writeln!(out)?;
    let c = &cal.canaries;
    writeln!(
        out,
        "Canaries: caught {} of {} planted defects ({}), passed {} of {} clean controls",
        c.caught,
        c.defects,
        percent(c.catch_rate),
        c.passed,
        c.controls
    )?;
    for tier in &cal.tiers {
        writeln!(
            out,
            "Tier {}: {:?}. {} audits, {} disagreed{}, {} escapes in 30 days",
            tier.tier,
            tier.state,
            tier.audits,
            tier.disagreements,
            tier.disagreement_upper
                .map(|u| format!(" (upper bound {:.0}%)", u * 100.0))
                .unwrap_or_default(),
            tier.escapes
        )?;
        for why in &tier.why {
            writeln!(out, "    {why}")?;
        }
    }
    let windows = db.with_conn(t::list_holds)?.len();
    if windows > 0 {
        writeln!(
            out,
            "\n{windows} tier 2 tasks are inside their veto window: hq verify holds"
        )?;
    }
    if cal.open_audits > 0 {
        writeln!(
            out,
            "\n{} audits are waiting for you: hq verify audits",
            cal.open_audits
        )?;
    }
    Ok(())
}

fn print_briefing(b: &t::Briefing, hours: i64, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "While you were away (last {hours} hours)\n")?;
    if b.mode == hq_core::config::VerificationMode::Off {
        writeln!(
            out,
            "Verification is off, so everything in review waits on you. Turn it on in the config: docs/TASK_VERIFICATION.md"
        )?;
        return Ok(());
    }
    if b.needs_you_total == 0 {
        writeln!(out, "Nothing needs you.")?;
    } else if let Some(offer) = &b.bulk_offer {
        writeln!(
            out,
            "{} need you.\n  {}\n  Run: {}",
            b.needs_you_total, offer.text, offer.command
        )?;
    } else {
        writeln!(out, "{} need you:", b.needs_you_total)?;
        for i in &b.needs_you {
            writeln!(out, "  {}: {}\n      {}", i.display_id, i.title, i.reason)?;
        }
    }
    if b.completed_total > 0 {
        writeln!(out, "\nHQ verified and completed {}:", b.completed_total)?;
        for g in &b.completed {
            let name = if g.initiative.is_empty() {
                "other"
            } else {
                g.initiative.as_str()
            };
            let levels: Vec<String> = g.by_level.iter().map(|(l, n)| format!("{n} {l}")).collect();
            let tier2 = if g.tier2 > 0 {
                format!(", {} tier 2", g.tier2)
            } else {
                String::new()
            };
            writeln!(
                out,
                "  {:>4}  {name} ({}{tier2})",
                g.count,
                levels.join(", ")
            )?;
        }
    }
    if b.reworked_total > 0 {
        writeln!(
            out,
            "\n{} failed verification and went back to their workers:",
            b.reworked_total
        )?;
        for r in &b.reworked {
            writeln!(
                out,
                "  {}: {}  ({} returns) {}",
                r.display_id, r.title, r.returns, r.last_finding
            )?;
        }
    }
    if !b.veto_windows.is_empty() {
        writeln!(out, "\nTier 2, inside the veto window (hq verify holds):")?;
        for w in &b.veto_windows {
            writeln!(out, "  {}: {}", w.display_id, w.title)?;
        }
    }
    writeln!(
        out,
        "\nAudit: {} completed tasks nobody has checked, {} waiting for your answer (hq verify pull, hq verify audits).",
        b.audit_pool, b.open_audits
    )?;
    for alert in &b.health.alerts {
        writeln!(out, "\nAttention: {alert}.")?;
    }
    Ok(())
}

fn queue(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let h = db.with_conn(|c| t::queue_health(c, cfg))?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&h)?)?;
        return Ok(());
    }
    if h.backlog.is_empty() {
        writeln!(out, "No task is waiting on anyone.")?;
    } else {
        writeln!(out, "Waiting on:")?;
        for (who, n) in &h.backlog {
            let mark = if who == t::WAIT_OWNER { "  <- you" } else { "" };
            writeln!(out, "  {who:<9} {n}{mark}")?;
        }
    }
    if !h.owner_by_kind.is_empty() {
        let kinds: Vec<String> = h
            .owner_by_kind
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect();
        writeln!(out, "  of which for you: {}", kinds.join(", "))?;
    }
    if let Some(hours) = h.oldest_owner_hours {
        writeln!(out, "\nOldest item waiting on you: {} days", hours / 24)?;
    }
    writeln!(
        out,
        "Last 24 hours: {} verdicts, {} tasks completed by HQ{}",
        h.verdicts_24h,
        h.completed_24h,
        h.est_clear_hours
            .map(|e| format!("; the verifier queue clears in about {e} hours"))
            .unwrap_or_default()
    )?;
    match h.monthly_ceiling_usd {
        Some(c) => writeln!(
            out,
            "Reviewer spend this month: ${:.2} of ${c:.2}",
            h.spent_this_month_usd
        )?,
        None => writeln!(out, "No reviewer budget is set, so no model is called.")?,
    }
    for d in &h.deferrals {
        writeln!(
            out,
            "  deferred ({}): {} tasks, {}",
            d.waiting_for, d.tasks, d.reason
        )?;
    }
    for alert in &h.alerts {
        writeln!(out, "\nAttention: {alert}.")?;
    }
    Ok(())
}

fn sweeps(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    opts: &Opts,
    out: &mut dyn Write,
) -> Result<()> {
    let action = opts.arg.as_deref().unwrap_or("list");
    let id = opts.extra.clone();
    let need_id = || {
        id.clone()
            .ok_or_else(|| anyhow::anyhow!("usage: hq verify sweep {action} <sweep-id>"))
    };
    match action {
        "list" => {
            let all = db.with_conn(|c| t::list_sweeps(c, 20))?;
            if opts.json {
                writeln!(out, "{}", serde_json::to_string_pretty(&all)?)?;
                return Ok(());
            }
            if all.is_empty() {
                writeln!(out, "No sweeps yet: hq verify sweep plan")?;
            }
            for s in &all {
                writeln!(
                    out,
                    "{}  {:<9} {} tasks  {}",
                    s.id,
                    s.status,
                    s.plan["total"].as_i64().unwrap_or(0),
                    s.created_at
                )?;
            }
            Ok(())
        }
        "plan" => {
            let scope = match (&opts.initiative, &opts.since) {
                (Some(_), Some(_)) => bail!("give --initiative or --since, not both"),
                (Some(initiative), None) => t::SweepScope::Initiative {
                    id: initiative.clone(),
                },
                (None, Some(date)) => t::SweepScope::Since { date: date.clone() },
                (None, None) => t::SweepScope::All,
            };
            let sweep = db.with_conn(|c| hq_tools::tasks::verify::start_sweep(c, cfg, &scope))?;
            writeln!(
                out,
                "Sweep {} started over {} tasks. The daemon reads each one's evidence without a model \
                 and prints the plan within a few minutes: hq verify sweep show {}",
                sweep.id,
                db.with_conn(|c| t::sweep_items(c, &sweep.id))?.len(),
                sweep.id
            )?;
            Ok(())
        }
        "show" => show_sweep(db, cfg, &need_id()?, opts.json, out),
        "run" | "resume" | "pause" | "cancel" | "apply" => {
            let id = need_id()?;
            let sweep = db.with_conn(|c| {
                hq_tools::tasks::verify::sweep_action(c, cfg, &id, action, opts.confirm)
            })?;
            let next = match sweep.status.as_str() {
                t::SWEEP_RUNNING => {
                    "The daemon verifies it a few tasks a minute; nothing moves until you apply."
                }
                t::SWEEP_PAUSED => "Paused between tasks; run it again to continue.",
                t::SWEEP_CANCELLED => "Cancelled. Nothing was moved.",
                t::SWEEP_APPLYING => "Applying: the daemon checks each task again and moves it.",
                _ => "",
            };
            writeln!(out, "Sweep {id} is {}. {next}", sweep.status)?;
            Ok(())
        }
        "undo" => {
            let id = need_id()?;
            let report = db.with_conn(|c| hq_tools::tasks::verify::undo_sweep(c, &id))?;
            writeln!(out, "Restored {} tasks to review.", report.restored)?;
            for (task, why) in &report.left_alone {
                writeln!(out, "  left alone: {task}, {why}")?;
            }
            Ok(())
        }
        other => bail!(
            "unknown sweep verb '{other}': plan, list, show, run, pause, resume, cancel, apply, undo"
        ),
    }
}

fn show_sweep(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    id: &str,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let sweep = db
        .with_conn(|c| t::get_sweep(c, id))?
        .ok_or_else(|| anyhow::anyhow!("no sweep '{id}'"))?;
    let items = db.with_conn(|c| t::sweep_items(c, id))?;
    let progress = db.with_conn(|c| t::sweep_summary(c, id))?;
    if json {
        let value = serde_json::json!({ "sweep": sweep, "progress": progress, "items": items });
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    writeln!(
        out,
        "Sweep {}: {} ({} tasks)",
        sweep.id,
        sweep.status,
        items.len()
    )?;
    let states = &progress["states"];
    writeln!(
        out,
        "  progress: {} unplanned, {} planned, {} with a verdict, {} applied",
        states["unplanned"].as_i64().unwrap_or(0),
        states["planned"].as_i64().unwrap_or(0),
        states["verdict"].as_i64().unwrap_or(0),
        states["applied"].as_i64().unwrap_or(0)
    )?;
    if sweep.plan.is_object() && !sweep.plan["buckets"].is_null() {
        writeln!(out, "\nBuckets:")?;
        for (bucket, label) in BUCKETS {
            let n = sweep.plan["buckets"][bucket].as_i64().unwrap_or(0);
            if n > 0 {
                writeln!(out, "  {n:>4}  {label}")?;
            }
        }
        let e = &sweep.plan["estimate"];
        if e["reviewer_runs"].as_i64().unwrap_or(0) > 0 {
            writeln!(
                out,
                "\nReviewer runs it needs: {} (about ${} and {} minutes; {})",
                e["reviewer_runs"],
                e["usd"],
                e["minutes"],
                e["basis"].as_str().unwrap_or("")
            )?;
            if !e["reviewer_enabled"].as_bool().unwrap_or(false) {
                writeln!(
                    out,
                    "  No reviewer budget is set (tasks.verification.reviewer_monthly_ceiling_usd), so those will wait."
                )?;
            }
        }
        writeln!(
            out,
            "{} would wait on you.",
            sweep.plan["would_wait_on_you"].as_i64().unwrap_or(0)
        )?;
    }
    for (bucket, label) in BUCKETS {
        let mine: Vec<&t::SweepItem> = items
            .iter()
            .filter(|i| i.bucket == *bucket)
            .take(5)
            .collect();
        if mine.is_empty() || sweep.status == t::SWEEP_PLANNING {
            continue;
        }
        writeln!(out, "\n{label} (first {}):", mine.len())?;
        for i in mine {
            writeln!(
                out,
                "  {}: {}  [{}] {}",
                i.display_id, i.title, i.state, i.finding
            )?;
        }
    }
    match sweep.status.as_str() {
        t::SWEEP_PLANNED => writeln!(out, "\nNext: hq verify sweep run {id}")?,
        t::SWEEP_READY => {
            let p = db.with_conn(|c| t::apply_preview(c, cfg, id))?;
            writeln!(
                out,
                "\nApplying moves {} tasks and leaves {} where they are.{}",
                p.will_move,
                p.will_keep,
                if p.needs_confirmation {
                    format!(" That is over {}: add --confirm.", p.confirm_over)
                } else {
                    String::new()
                }
            )?;
            writeln!(out, "Next: hq verify sweep apply {id}")?;
        }
        t::SWEEP_APPLIED => writeln!(out, "\nTo reverse it: hq verify sweep undo {id}")?,
        _ => {}
    }
    Ok(())
}

const BUCKETS: &[(&str, &str)] = &[
    (t::BUCKET_PASSED, "passed: complete"),
    (t::BUCKET_NEEDS_REVIEW, "need a reviewer's judgement"),
    (t::BUCKET_FAILED, "failed: back to to_do with the findings"),
    (
        t::BUCKET_NO_ARTIFACT,
        "no artifact: back to to_do with what to link",
    ),
    (t::BUCKET_UNDECIDABLE, "undecidable: left in place"),
    (t::BUCKET_NEEDS_YOU, "need you: stay in your list"),
];

fn holds(db: &Database, json: bool, out: &mut dyn Write) -> Result<()> {
    let open = db.with_conn(t::list_holds)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&open)?)?;
        return Ok(());
    }
    if open.is_empty() {
        writeln!(out, "No tier 2 task is inside a veto window.")?;
        return Ok(());
    }
    for h in &open {
        let task = db.with_conn(|c| t::get_task(c, &h.task_id))?;
        let (display, title) = task.map_or_else(
            || (h.task_id.clone(), String::new()),
            |task| (task.display_id, task.title),
        );
        let at = chrono::DateTime::from_timestamp(h.release_at, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        writeln!(
            out,
            "{display}: {title}\n    completes at {at} UTC unless vetoed"
        )?;
    }
    writeln!(out, "\nhq verify veto <task> --note \"what is wrong\"")?;
    Ok(())
}

fn audits(db: &Database, json: bool, out: &mut dyn Write) -> Result<()> {
    let open = db.with_conn(|c| t::open_audits(c, 200))?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&open)?)?;
        return Ok(());
    }
    if open.is_empty() {
        writeln!(out, "No audits are waiting for you.")?;
        return Ok(());
    }
    for a in &open {
        let task = db.with_conn(|c| t::get_task(c, &a.task_id))?;
        let (display, title) = task.map_or_else(
            || (a.task_id.clone(), String::new()),
            |task| (task.display_id, task.title),
        );
        let verdict = db.with_conn(|c| t::latest_verdict(c, &a.task_id))?;
        writeln!(out, "{}  {display}: {title}", a.id)?;
        if let Some(v) = verdict {
            writeln!(
                out,
                "    tier {}, {} evidence by {}, artifact {}",
                v.tier, v.level, v.verifier, v.artifact
            )?;
        }
    }
    writeln!(
        out,
        "\nhq verify agree <id>   or   hq verify disagree <id> --note \"what is wrong\""
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(sub: &str, arg: Option<&str>) -> Opts {
        Opts {
            sub: sub.into(),
            arg: arg.map(String::from),
            ..Default::default()
        }
    }

    fn try_render(db: &Database, o: &Opts) -> Result<String> {
        let mut out = Vec::new();
        render(
            db,
            &hq_core::config::VerificationConfig::default(),
            o,
            &mut out,
        )?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn run_to_string(db: &Database, sub: &str, arg: Option<&str>, note: Option<&str>) -> String {
        let mut o = opts(sub, arg);
        o.note = note.map(String::from);
        try_render(db, &o).unwrap()
    }

    #[test]
    fn a_fresh_install_says_insufficient_data_and_waits_on_no_one() {
        let db = Database::open_memory().unwrap();
        let text = run_to_string(&db, "status", None, None);
        assert!(text.contains("No task is waiting on anyone."), "{text}");
        assert!(text.contains("Tier 1: Insufficient"), "{text}");
        assert!(text.contains("0 of 30 audits answered"), "{text}");
        assert!(text.contains("caught 0 of 0"), "{text}");
    }

    #[test]
    fn nothing_to_pull_and_nothing_to_answer_are_said_plainly() {
        let db = Database::open_memory().unwrap();
        assert!(run_to_string(&db, "pull", Some("5"), None).contains("Nothing to audit"));
        assert!(run_to_string(&db, "audits", None, None).contains("No audits are waiting"));
        let err = try_render(&db, &opts("agree", None)).unwrap_err();
        assert!(err.to_string().contains("usage: hq verify agree"), "{err}");
        let err = try_render(&db, &opts("pull", Some("many"))).unwrap_err();
        assert!(err.to_string().contains("number of audits"), "{err}");
    }

    #[test]
    fn a_veto_needs_a_task_with_an_open_window() {
        let db = Database::open_memory().unwrap();
        assert!(run_to_string(&db, "holds", None, None).contains("No tier 2 task"));
        let err = try_render(&db, &opts("veto", None)).unwrap_err();
        assert!(err.to_string().contains("usage: hq verify veto"), "{err}");
        let err = try_render(&db, &opts("veto", Some("TK-9"))).unwrap_err();
        assert!(err.to_string().contains("no task"), "{err}");
    }

    #[test]
    fn rollout_on_an_install_with_verification_off_offers_shadow() {
        let db = Database::open_memory().unwrap();
        let text = run_to_string(&db, "rollout", None, None);
        assert!(text.contains("Rollout: Off: nothing is verified"), "{text}");
        assert!(
            text.contains("Next: Turn verification on in shadow mode"),
            "{text}"
        );
        assert!(
            text.contains("Every condition is met. The change is: tasks.verification.mode: shadow"),
            "{text}"
        );
    }

    #[test]
    fn rollout_in_shadow_lists_what_is_missing_and_the_setting_that_takes_the_step() {
        let db = Database::open_memory().unwrap();
        let cfg = hq_core::config::VerificationConfig {
            mode: hq_core::config::VerificationMode::Shadow,
            ..Default::default()
        };
        let mut out = Vec::new();
        render(&db, &cfg, &opts("rollout", None), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Next: Enforce tier 0"), "{text}");
        assert!(
            text.contains("[ ] Enough tier 0 verdicts compared"),
            "{text}"
        );
        assert!(text.contains("0 of 30 compared"), "{text}");
        assert!(text.contains("[ ] A reviewer is configured"), "{text}");
        assert!(text.contains("conditions are not met yet"), "{text}");
        assert!(text.contains("enforce_tiers: [0]"), "{text}");

        let mut o = opts("rollout", None);
        o.json = true;
        let mut out = Vec::new();
        render(&db, &cfg, &o, &mut out).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stage"], "shadow");
        assert_eq!(v["ready"], false);
        assert_eq!(v["shadow"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn the_help_lists_every_verb() {
        let db = Database::open_memory().unwrap();
        let help = run_to_string(&db, "help", None, None);
        for verb in [
            "status", "rollout", "audits", "pull", "agree", "disagree", "holds", "veto", "sweep",
            "briefing", "queue",
        ] {
            assert!(help.contains(verb), "{verb} missing from:\n{help}");
        }
    }

    #[test]
    fn the_briefing_and_queue_read_plainly_on_an_empty_install() {
        let db = Database::open_memory().unwrap();
        let text = run_to_string(&db, "briefing", None, None);
        assert!(text.contains("Verification is off"), "{text}");
        let queue = run_to_string(&db, "queue", None, None);
        assert!(queue.contains("No task is waiting on anyone."), "{queue}");
        assert!(queue.contains("No reviewer budget is set"), "{queue}");
        let err = try_render(&db, &opts("briefing", Some("soon"))).unwrap_err();
        assert!(err.to_string().contains("number of hours"), "{err}");
    }

    #[test]
    fn an_unknown_audit_is_an_error_not_a_silent_agree() {
        let db = Database::open_memory().unwrap();
        let err = try_render(&db, &opts("agree", Some("au-nope"))).unwrap_err();
        assert!(err.to_string().contains("no audit"), "{err}");
    }

    #[test]
    fn a_sweep_cannot_start_while_verification_is_off_and_an_empty_list_says_how_to_begin() {
        let db = Database::open_memory().unwrap();
        let text = try_render(
            &db,
            &Opts {
                sub: "sweep".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(text.contains("hq verify sweep plan"), "{text}");
        let mut o = opts("sweep", Some("plan"));
        o.initiative = Some("in-1".into());
        o.since = Some("2026-01-01".into());
        assert!(
            try_render(&db, &o)
                .unwrap_err()
                .to_string()
                .contains("not both")
        );
        let mut o = opts("sweep", Some("plan"));
        let err = try_render(&db, &o).unwrap_err();
        assert!(err.to_string().contains("verification is off"), "{err}");
        o.arg = Some("show".into());
        assert!(
            try_render(&db, &o)
                .unwrap_err()
                .to_string()
                .contains("usage: hq verify sweep show")
        );
        o.arg = Some("show".into());
        o.extra = Some("sw-nope".into());
        assert!(
            try_render(&db, &o)
                .unwrap_err()
                .to_string()
                .contains("no sweep")
        );
        o.arg = Some("explode".into());
        assert!(
            try_render(&db, &o)
                .unwrap_err()
                .to_string()
                .contains("unknown sweep verb")
        );
    }
}
