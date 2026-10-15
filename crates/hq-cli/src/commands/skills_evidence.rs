//! `hq skills evidence`: what the skill invocation records say, and `purge` to delete them.
//!
//! Owner's CLI only. There is no tool, MCP route or web endpoint for either, so an agent working
//! through HQ cannot read or delete this evidence; the records hold ids and fixed labels, never
//! prompts or documents (see `docs/SKILL_FLYWHEEL.md`).

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use hq_core::config::HqConfig;
use hq_db::skill_evidence::{
    MIN_INVOCATIONS, MIN_SESSIONS, Pattern, PatternKind, SkillEvidence, VersionStats, WINDOW_DAYS,
};
use hq_db::skill_invocations::Purge;

/// Enough of a skill hash to tell versions apart by eye.
const HASH_CHARS: usize = 12;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct EvidenceArgs {
    /// One skill; every skill used in the window when left out
    pub name: Option<String>,
    /// Days of history to read
    #[arg(long, default_value_t = WINDOW_DAYS)]
    pub days: u32,
    /// Write a proposal to skills/_proposed/ for each skill with enough evidence
    #[arg(long)]
    pub propose: bool,
    /// Print JSON instead of text
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<EvidenceCommand>,
}

#[derive(Subcommand)]
pub enum EvidenceCommand {
    /// Delete skill telemetry by record id or by a date range. Shows what would go; add --yes
    /// to delete. Skills and their audit trail are never touched.
    Purge {
        /// One invocation, by its id (inv_...)
        #[arg(long, conflicts_with_all = ["skill", "from", "before"])]
        id: Option<String>,
        /// Only this skill's records
        #[arg(long)]
        skill: Option<String>,
        /// From this date (YYYY-MM-DD, inclusive)
        #[arg(long)]
        from: Option<String>,
        /// Up to this date (YYYY-MM-DD, exclusive)
        #[arg(long)]
        before: Option<String>,
        /// Really delete; without it nothing changes
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(config: &HqConfig, args: EvidenceArgs, output: &mut dyn Write) -> Result<()> {
    let db = hq_db::Database::open_existing(&config.db_path())
        .context("there is no HQ database yet, so there is no skill evidence")?;
    match args.command {
        Some(EvidenceCommand::Purge {
            id,
            skill,
            from,
            before,
            yes,
        }) => purge(&db, id, skill, from, before, yes, output),
        None => report(config, &db, &args, output),
    }
}

fn purge(
    db: &hq_db::Database,
    id: Option<String>,
    skill: Option<String>,
    from: Option<String>,
    before: Option<String>,
    yes: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let scope = match &id {
        Some(id) => Purge::Record(id),
        None => Purge::Range {
            skill: skill.as_deref(),
            from: from.as_deref(),
            before: before.as_deref(),
        },
    };
    // Run it either way and undo it unless asked to keep it, so the preview is exact.
    let purged = db.with_conn(|conn| {
        conn.execute_batch("SAVEPOINT purge_preview")?;
        let result = hq_db::skill_invocations::purge(conn, scope);
        let finish = if yes && result.is_ok() {
            "RELEASE purge_preview"
        } else {
            "ROLLBACK TO purge_preview; RELEASE purge_preview"
        };
        conn.execute_batch(finish)?;
        result
    })?;
    if yes {
        writeln!(
            output,
            "Deleted {} skill record(s) and {} monthly roll-up(s). Skills and their audit trail are untouched.",
            purged.records, purged.rollups
        )?;
    } else {
        writeln!(
            output,
            "Would delete {} skill record(s) and {} monthly roll-up(s). Nothing was changed; add --yes to delete.",
            purged.records, purged.rollups
        )?;
    }
    Ok(())
}

fn short(hash: &Option<String>) -> String {
    hash.as_deref().map_or("unrecorded".to_string(), |h| {
        h.chars().take(HASH_CHARS).collect()
    })
}

fn mean(value: Option<f64>) -> String {
    value.map_or("none".to_string(), |v| format!("{v:.2}"))
}

fn version_line(v: &VersionStats) -> String {
    format!(
        "{}  {} loads, {} sessions; outcomes {} accepted, {} reworked, {} rejected, {} not settled; \
         mean {} over {} scored; verdicts {} helped, {} hurt, {} unused",
        short(&v.skill_hash),
        v.invocations,
        v.sessions,
        v.accepted,
        v.reworked,
        v.rejected,
        v.no_signal,
        mean(v.mean_score),
        v.scored,
        v.helped,
        v.hurt,
        v.unused
    )
}

fn pattern_line(p: &Pattern) -> String {
    let what = match p.kind {
        PatternKind::ReworkedForPurpose => format!("{} ({})", p.kind.as_str(), p.qualifier),
        _ => p.kind.as_str().to_string(),
    };
    let size = format!(
        "{} loads across {} sessions, {} to {}, version {}",
        p.invocations,
        p.sessions,
        p.first,
        p.last,
        short(&p.skill_hash)
    );
    if p.enough {
        let ids: Vec<&str> = p.examples.iter().map(|e| e.id.as_str()).collect();
        format!("FINDING {what}: {size}; examples {}", ids.join(", "))
    } else {
        format!(
            "too small to count, {what}: {size} (needs {MIN_INVOCATIONS} loads across {MIN_SESSIONS} sessions)"
        )
    }
}

fn write_text(
    evidence: &[SkillEvidence],
    proposals: &[(String, Option<std::path::PathBuf>)],
    days: u32,
    output: &mut dyn Write,
) -> Result<()> {
    if evidence.is_empty() {
        writeln!(output, "No skill was loaded in the last {days} days.")?;
        return Ok(());
    }
    writeln!(output, "Skill evidence for the last {days} days\n")?;
    for e in evidence {
        writeln!(output, "{}", e.skill)?;
        writeln!(
            output,
            "  {} loads in {} sessions",
            e.invocations, e.sessions
        )?;
        for v in &e.versions {
            writeln!(output, "  version {}", version_line(v))?;
        }
        for c in &e.comparisons {
            writeln!(
                output,
                "  {} -> {}: mean {} ({} loads) -> {} ({} loads){}",
                short(&c.before),
                short(&c.after),
                mean(c.before_mean),
                c.before_invocations,
                mean(c.after_mean),
                c.after_invocations,
                if c.small_sample {
                    ". Small sample: anecdotal, and a correlation at best."
                } else {
                    ". A correlation, not proof the change caused it."
                }
            )?;
        }
        for p in &e.patterns {
            writeln!(output, "  {}", pattern_line(p))?;
        }
        if !e.sufficient() {
            writeln!(
                output,
                "  insufficient evidence: no pattern reaches {MIN_INVOCATIONS} loads across {MIN_SESSIONS} sessions, so there is nothing to propose"
            )?;
        }
        if let Some((_, path)) = proposals.iter().find(|(name, _)| name == &e.skill) {
            match path {
                Some(path) => writeln!(output, "  proposal written to {}", path.display())?,
                None => writeln!(output, "  no proposal")?,
            }
        } else if e.sufficient() {
            writeln!(
                output,
                "  enough evidence for a proposal: run again with --propose"
            )?;
        }
        writeln!(output)?;
    }
    Ok(())
}

fn auto_loading(skills_dir: &Path) -> BTreeSet<String> {
    hq_tools::skills::list_skills(skills_dir)
        .into_iter()
        .filter(|s| s.auto_load && !s.hints.is_empty())
        .map(|s| s.name)
        .collect()
}

fn report(
    config: &HqConfig,
    db: &hq_db::Database,
    args: &EvidenceArgs,
    output: &mut dyn Write,
) -> Result<()> {
    let skills_dir = hq_core::skills_dir(&config.vault_path);
    if args.days == 0 || args.days > 3650 {
        bail!("--days must be between 1 and 3650");
    }
    let auto = auto_loading(&skills_dir);
    // Settle what the work has shown since the daemon last did, so the report is current.
    let evidence = db.with_conn(|conn| {
        hq_db::skill_outcomes::settle(conn, args.days.max(30))?;
        hq_db::skill_evidence::evidence(conn, args.name.as_deref(), args.days, &auto)
    })?;

    let mut proposals: Vec<(String, Option<std::path::PathBuf>)> = Vec::new();
    if args.propose {
        for e in &evidence {
            let current = hq_tools::skill_audit::live_hash(&skills_dir, &e.skill);
            let path =
                hq_tools::skills::write_evidence_proposal(&skills_dir, e, current.as_deref())?;
            proposals.push((e.skill.clone(), path));
        }
    }

    if args.json {
        let proposed: Vec<serde_json::Value> = proposals
            .iter()
            .map(|(skill, path)| serde_json::json!({ "skill": skill, "path": path }))
            .collect();
        writeln!(
            output,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "window_days": args.days,
                "min_invocations": MIN_INVOCATIONS,
                "min_sessions": MIN_SESSIONS,
                "skills": evidence,
                "proposals": proposed,
            }))?
        )?;
        return Ok(());
    }
    write_text(&evidence, &proposals, args.days, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::skill_invocations::InvocationTrigger;
    use hq_db::tasks as t;

    const SKILL: &str = "---\nname: sheets\ndescription: Work with spreadsheets\nmanaged: true\nhints:\n  - spreadsheet\n---\n\n# Sheets\n\n1. Back up the original file.\n2. Edit the copy.\n3. Confirm before overwriting.\n";

    struct Fx {
        vault: tempfile::TempDir,
        config: HqConfig,
        db: hq_db::Database,
    }

    impl Fx {
        fn new() -> Self {
            let vault = tempfile::tempdir().unwrap();
            let skills = vault.path().join("skills");
            std::fs::create_dir_all(skills.join("sheets")).unwrap();
            std::fs::write(skills.join("sheets/SKILL.md"), SKILL).unwrap();
            let config = HqConfig {
                vault_path: vault.path().to_path_buf(),
                ..Default::default()
            };
            let db = hq_db::Database::open(&config.db_path()).unwrap();
            db.with_conn(|c| {
                t::create_initiative(c, "in-1", "personal", None, "Work", "work", "WRK")?;
                Ok(())
            })
            .unwrap();
            Self { vault, config, db }
        }

        fn skills(&self) -> std::path::PathBuf {
            self.vault.path().join("skills")
        }

        fn move_to(&self, task: &str, status: &str) {
            self.db
                .with_conn(|c| {
                    t::update_task(
                        c,
                        task,
                        &t::TaskPatch {
                            status: Some(status.to_string()),
                            ..Default::default()
                        },
                        None,
                    )
                    .map(|_| ())
                })
                .unwrap();
        }

        /// A session loads the skill under a task of its own, exits cleanly, and the work is
        /// reviewed and sent back.
        fn reopened_use(&self, n: usize, session: &str) {
            let task = format!("tk-{n}");
            self.db
                .with_conn(|c| {
                    t::create_task(
                        c,
                        &task,
                        "in-1",
                        &t::NewTask {
                            title: "A task",
                            tags: &["research".to_string()],
                            created_by: "test",
                            ..Default::default()
                        },
                    )
                    .map(|_| ())
                })
                .unwrap();
            self.move_to(&task, "in_progress");
            hq_tools::skills::log_skill_load(
                &self.db,
                &self.skills(),
                "sheets",
                session,
                InvocationTrigger::AutoLoad,
                Some(&task),
            )
            .unwrap();
            self.db
                .with_conn(|c| {
                    hq_db::skill_invocations::update_outcome_by_session(
                        c,
                        session,
                        1.0,
                        "session_result",
                    )
                })
                .unwrap();
            self.move_to(&task, "ready_for_review");
            self.move_to(&task, "in_progress");
        }

        fn args(&self, propose: bool, json: bool) -> EvidenceArgs {
            EvidenceArgs {
                name: None,
                days: WINDOW_DAYS,
                propose,
                json,
                command: None,
            }
        }

        fn run(&self, args: EvidenceArgs) -> String {
            let mut out = Vec::new();
            run(&self.config, args, &mut out).unwrap();
            String::from_utf8(out).unwrap()
        }
    }

    #[test]
    fn from_a_skill_use_to_a_reviewed_change_nothing_is_rewritten_on_the_way() {
        let fx = Fx::new();
        std::fs::write(
            fx.skills().join("sheets/fixtures.yaml"),
            "triggers:\n  - fix this spreadsheet\nrequired_steps:\n  - back up the original file\n  - confirm before overwriting\n",
        )
        .unwrap();
        // The skill's audit trail, which none of this may touch.
        std::fs::write(fx.skills().join("_audit.jsonl"), "{\"name\":\"sheets\"}\n").unwrap();
        let skill_before = std::fs::read_to_string(fx.skills().join("sheets/SKILL.md")).unwrap();
        let audit_before = std::fs::read_to_string(fx.skills().join("_audit.jsonl")).unwrap();

        // Six uses over four sessions: each exited cleanly and each task was sent back.
        for (n, session) in ["s1", "s1", "s2", "s3", "s4", "s4"].iter().enumerate() {
            fx.reopened_use(n, session);
        }

        // 1. The evidence: the clean exits did not count as success, and the pattern is a finding.
        let text = fx.run(fx.args(false, false));
        assert!(text.contains("sheets"), "{text}");
        assert!(text.contains("6 loads in 4 sessions"), "{text}");
        assert!(
            text.contains("FINDING reworked_outcomes: 6 loads across 4 sessions"),
            "{text}"
        );
        assert!(
            text.contains("mean 0.50 over 6 scored"),
            "exit scores were replaced: {text}"
        );
        assert!(text.contains("enough evidence for a proposal"), "{text}");
        assert!(
            !fx.skills().join("_proposed").exists(),
            "a report alone writes nothing"
        );

        // 2. The proposal: sourced by ids, and not a change.
        let text = fx.run(fx.args(true, false));
        let proposal = fx.skills().join("_proposed/sheets/PROPOSAL.md");
        assert!(text.contains("proposal written to"), "{text}");
        let proposal_text = std::fs::read_to_string(&proposal).unwrap();
        assert!(
            proposal_text.contains("6 loads across 4 sessions"),
            "{proposal_text}"
        );
        assert!(
            proposal_text.contains("`inv_"),
            "examples are ids: {proposal_text}"
        );
        assert!(
            proposal_text.contains("outcome from `task:tk-"),
            "{proposal_text}"
        );
        assert!(
            proposal_text.contains("2 required-step"),
            "the fixtures are named: {proposal_text}"
        );
        assert_eq!(
            std::fs::read_to_string(fx.skills().join("sheets/SKILL.md")).unwrap(),
            skill_before,
            "the skill is untouched"
        );
        assert!(hq_tools::skills::approve_proposed_skill(&fx.skills(), "sheets").is_err());

        // 3. A change is reviewed against the regression fixtures before it lands: one that
        // drops a required step is refused and reverted, one that keeps them goes through.
        let drops_a_step = hq_tools::skill_edit::SkillEdit {
            patch: Some(("3. Confirm before overwriting.\n", "")),
            ..Default::default()
        };
        let refused = hq_tools::skill_edit::edit_skill(&fx.skills(), "sheets", &drops_a_step);
        assert!(
            refused.is_err(),
            "a change that drops a required step is not applied"
        );
        assert_eq!(
            std::fs::read_to_string(fx.skills().join("sheets/SKILL.md")).unwrap(),
            skill_before,
            "and the skill is as it was"
        );
        let keeps_them = hq_tools::skill_edit::SkillEdit {
            patch: Some((
                "3. Confirm before overwriting.\n",
                "3. Confirm before overwriting.\n4. Check the totals still add up.\n",
            )),
            ..Default::default()
        };
        hq_tools::skill_edit::edit_skill(&fx.skills(), "sheets", &keeps_them).unwrap();
        assert!(hq_tools::skills::skill_failures(&fx.skills(), "sheets").is_empty());

        // 4. Later loads record the new version, and the report compares the two honestly.
        fx.reopened_use(10, "s9");
        let text = fx.run(fx.args(false, false));
        assert!(
            text.contains("Small sample"),
            "one load on the new version is anecdote: {text}"
        );

        // Nothing in all this touched the audit trail.
        assert_eq!(
            std::fs::read_to_string(fx.skills().join("_audit.jsonl")).unwrap(),
            audit_before
        );
    }

    #[test]
    fn a_thin_record_is_reported_as_insufficient_and_makes_no_proposal() {
        let fx = Fx::new();
        for (n, session) in ["s1", "s2", "s3"].iter().enumerate() {
            fx.reopened_use(n, session);
        }
        let text = fx.run(fx.args(true, false));
        assert!(text.contains("insufficient evidence"), "{text}");
        assert!(
            text.contains("too small to count"),
            "the small pattern is shown with its size: {text}"
        );
        assert!(!text.contains("FINDING"), "{text}");
        assert!(text.contains("no proposal"), "{text}");
        assert!(!fx.skills().join("_proposed").exists());
    }

    #[test]
    fn json_output_carries_the_thresholds_and_the_same_numbers() {
        let fx = Fx::new();
        for (n, session) in ["s1", "s2", "s3", "s3", "s1"].iter().enumerate() {
            fx.reopened_use(n, session);
        }
        let out = fx.run(fx.args(false, true));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["min_invocations"], 5);
        assert_eq!(v["min_sessions"], 3);
        assert_eq!(v["skills"][0]["skill"], "sheets");
        assert_eq!(v["skills"][0]["patterns"][0]["enough"], true);
    }

    #[test]
    fn a_window_with_no_loads_says_so() {
        let fx = Fx::new();
        assert!(
            fx.run(fx.args(false, false))
                .contains("No skill was loaded")
        );
    }

    #[test]
    fn purge_shows_what_it_would_remove_and_deletes_only_when_told_to() {
        let fx = Fx::new();
        fx.reopened_use(1, "s1");
        fx.reopened_use(2, "s2");
        std::fs::write(fx.skills().join("_audit.jsonl"), "{\"name\":\"sheets\"}\n").unwrap();
        let ids: Vec<String> = fx
            .db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT id FROM skill_invocations ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap();
        let count = |fx: &Fx| -> i64 {
            fx.db
                .with_conn(|c| {
                    Ok(c.query_row("SELECT COUNT(*) FROM skill_invocations", [], |r| r.get(0))?)
                })
                .unwrap()
        };
        let purge_args = |id: Option<String>, skill: Option<&str>, yes: bool| EvidenceArgs {
            name: None,
            days: WINDOW_DAYS,
            propose: false,
            json: false,
            command: Some(EvidenceCommand::Purge {
                id,
                skill: skill.map(str::to_string),
                from: None,
                before: None,
                yes,
            }),
        };

        // Without --yes: a count, and nothing changes.
        let out = fx.run(purge_args(Some(ids[0].clone()), None, false));
        assert!(out.contains("Would delete 1 skill record"), "{out}");
        assert_eq!(count(&fx), 2);

        // With --yes: that one record goes.
        let out = fx.run(purge_args(Some(ids[0].clone()), None, true));
        assert!(out.contains("Deleted 1 skill record"), "{out}");
        assert_eq!(count(&fx), 1);

        // By skill.
        fx.run(purge_args(None, Some("sheets"), true));
        assert_eq!(count(&fx), 0);

        // Skills and the audit trail are as they were.
        assert_eq!(
            std::fs::read_to_string(fx.skills().join("sheets/SKILL.md")).unwrap(),
            SKILL
        );
        assert_eq!(
            std::fs::read_to_string(fx.skills().join("_audit.jsonl")).unwrap(),
            "{\"name\":\"sheets\"}\n"
        );
    }

    #[test]
    fn a_purge_with_no_target_is_refused() {
        let fx = Fx::new();
        let args = EvidenceArgs {
            name: None,
            days: WINDOW_DAYS,
            propose: false,
            json: false,
            command: Some(EvidenceCommand::Purge {
                id: None,
                skill: None,
                from: None,
                before: None,
                yes: true,
            }),
        };
        assert!(run(&fx.config, args, &mut Vec::new()).is_err());
    }
}
