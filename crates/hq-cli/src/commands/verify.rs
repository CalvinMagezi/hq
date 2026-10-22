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

pub async fn run(
    config: &HqConfig,
    sub: &str,
    arg: Option<String>,
    note: Option<String>,
    json: bool,
) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    render(
        &db,
        &config.tasks.verification,
        sub,
        arg,
        note,
        json,
        &mut std::io::stdout(),
    )
}

fn render(
    db: &Database,
    cfg: &hq_core::config::VerificationConfig,
    sub: &str,
    arg: Option<String>,
    note: Option<String>,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    match sub {
        "status" | "" => status(db, cfg, json, out),
        "audits" => audits(db, json, out),
        "holds" => holds(db, json, out),
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
                "Usage: hq verify [status|audits|pull [n]|agree <id>|disagree <id>|holds|veto <task>]\n"
            )?;
            for (verb, what) in HELP {
                writeln!(out, "  {verb:<10} {what}")?;
            }
            Ok(())
        }
    }
}

const HELP: [(&str, &str); 7] = [
    (
        "status",
        "Mode, who tasks are waiting on, and whether the verifier has earned its autonomy",
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
];

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

    fn run_to_string(db: &Database, sub: &str, arg: Option<&str>, note: Option<&str>) -> String {
        let mut out = Vec::new();
        render(
            db,
            &hq_core::config::VerificationConfig::default(),
            sub,
            arg.map(String::from),
            note.map(String::from),
            false,
            &mut out,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
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
        let mut out = Vec::new();
        let err = render(
            &db,
            &hq_core::config::VerificationConfig::default(),
            "agree",
            None,
            None,
            false,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("usage: hq verify agree"), "{err}");
        let err = render(
            &db,
            &hq_core::config::VerificationConfig::default(),
            "pull",
            Some("many".into()),
            None,
            false,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("number of audits"), "{err}");
    }

    #[test]
    fn a_veto_needs_a_task_with_an_open_window() {
        let db = Database::open_memory().unwrap();
        assert!(run_to_string(&db, "holds", None, None).contains("No tier 2 task"));
        let mut out = Vec::new();
        let cfg = hq_core::config::VerificationConfig::default();
        let err = render(&db, &cfg, "veto", None, None, false, &mut out).unwrap_err();
        assert!(err.to_string().contains("usage: hq verify veto"), "{err}");
        let err = render(
            &db,
            &cfg,
            "veto",
            Some("TK-9".into()),
            None,
            false,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no task"), "{err}");
    }

    #[test]
    fn the_help_lists_every_verb() {
        let db = Database::open_memory().unwrap();
        let help = run_to_string(&db, "help", None, None);
        for verb in [
            "status", "audits", "pull", "agree", "disagree", "holds", "veto",
        ] {
            assert!(help.contains(verb), "{verb} missing from:\n{help}");
        }
    }

    #[test]
    fn an_unknown_audit_is_an_error_not_a_silent_agree() {
        let db = Database::open_memory().unwrap();
        let mut out = Vec::new();
        let err = render(
            &db,
            &hq_core::config::VerificationConfig::default(),
            "agree",
            Some("au-nope".into()),
            None,
            false,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no audit"), "{err}");
    }
}
