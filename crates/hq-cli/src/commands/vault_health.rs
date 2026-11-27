//! `hq vault health` and the vault section of `hq doctor`.
//!
//! Both read what `hq_tools::vault_health` reads for the web and phone screens, so the same
//! numbers show everywhere. Nothing here changes a note.

use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::config::HqConfig;
use hq_vault::health::{Ignored, Report};

/// `hq vault health [--json] [--apply [--tier a|all]]`: check every note now and say what is
/// wrong, and with `--apply` fix what can be fixed. `--tier a` (the default) does only the
/// fixes HQ is certain about; `--tier all` adds the likely ones. The originals are saved first
/// and the run can be undone with `hq vault repair --undo <run>`.
pub fn run(config: &HqConfig, args: &[String], json: bool) -> Result<()> {
    if args.iter().any(|a| a == "--apply") {
        return apply(config, args, json);
    }
    let ignored = load_ignored(config);
    let report = hq_tools::vault_health::scan_now(&config.vault_path, &ignored);
    let plan = open_db(config)
        .and_then(|db| hq_tools::vault_health::plan_now(&config.vault_path, &db).ok())
        .map(|(plan, _)| plan);
    if json {
        let mut value = serde_json::to_value(&report)?;
        if let Some(plan) = &plan {
            value["fixes"] = serde_json::to_value(&plan.fixes)?;
            value["skipped"] = serde_json::to_value(&plan.skipped)?;
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print!("{}", render_text(&report));
        if let Some(plan) = &plan {
            print!("{}", render_fixes(plan));
        }
    }
    Ok(())
}

fn open_db(config: &HqConfig) -> Option<hq_db::Database> {
    hq_db::Database::open_existing(&config.db_path()).ok()
}

fn apply(config: &HqConfig, args: &[String], json: bool) -> Result<()> {
    let tier = args
        .iter()
        .position(|a| a == "--tier")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("a");
    let which = match tier.to_lowercase().as_str() {
        "a" => hq_tools::vault_health::Which::TierA,
        "all" | "b" => hq_tools::vault_health::Which::All,
        other => anyhow::bail!("--tier is a or all, not '{other}'"),
    };
    let Some(db) = open_db(config) else {
        anyhow::bail!("HQ has no database yet: start HQ once, then try again");
    };
    let db = std::sync::Arc::new(db);
    let applied = hq_tools::vault_health::apply_repairs(
        &config.vault_path,
        &db,
        &which,
        hq_db::vault_health::Source::Owner,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&applied)?);
    } else {
        print!("{}", render_applied(&applied));
    }
    Ok(())
}

/// What a repair run did, in plain lines.
pub fn render_applied(a: &hq_tools::vault_health::Applied) -> String {
    let d = &a.done;
    if d.fixes.is_empty() {
        let mut out = String::from("Nothing was fixed.\n");
        if !d.skipped.is_empty() {
            out.push_str(&skipped_lines(&d.skipped));
        }
        return out;
    }
    let mut out = format!(
        "Fixed {} thing(s): {} note(s) changed, {} removed.\n",
        d.fixes.len(),
        d.rewritten.len(),
        d.trashed.len()
    );
    out.push_str(&format!(
        "The originals are saved. To take it back: hq vault repair --undo {}\n",
        d.run_id
    ));
    if !d.skipped.is_empty() {
        out.push_str(&skipped_lines(&d.skipped));
    }
    out
}

fn why(reason: hq_vault::fixes::SkipReason) -> &'static str {
    use hq_vault::fixes::SkipReason::*;
    match reason {
        Protected => "HQ's own folders are never changed",
        MachineWritten => "written by HQ's own upkeep",
        Suite => "a sheet, deck or dashboard",
        Pinned => "pinned, so only you change it",
        AgentUnreviewed => "written by an assistant and not reviewed yet",
        RecentlyEdited => "edited a moment ago",
        TooLarge => "too large to change safely",
        CannotRelink => "links to it cannot be pointed at the other note",
        Changed => "changed since the check",
        Conflict => "two fixes wanted the same words",
    }
}

fn skipped_lines(skipped: &[hq_vault::fixes::Skipped]) -> String {
    let mut out = format!("Left alone: {}\n", skipped.len());
    for s in skipped.iter().take(10) {
        out.push_str(&format!("  {}  ({})\n", s.note, why(s.why)));
    }
    if skipped.len() > 10 {
        out.push_str(&format!("  and {} more\n", skipped.len() - 10));
    }
    out
}

/// What can be fixed, as lines under the report.
pub fn render_fixes(plan: &hq_vault::fixes::Plan) -> String {
    use hq_vault::fixes::{Kind, Tier};
    let (a, b) = plan.counts();
    if a + b == 0 && plan.skipped.is_empty() {
        return String::new();
    }
    let mut out = format!("\nCan be fixed: {a} certain, {b} likely\n");
    for f in plan.fixes.iter().take(10) {
        let tier = if f.tier == Tier::A {
            "certain"
        } else {
            "likely "
        };
        let what = match f.kind {
            Kind::NestedFrontmatter => format!("{}  merge the second block of properties", f.note),
            Kind::MergeDuplicate => format!(
                "{}  same text as {}: remove it, point links there",
                f.note, f.after
            ),
            _ => format!(
                "{}:{}  {}  becomes  {}",
                f.note,
                f.line.unwrap_or(0),
                f.before,
                f.after
            ),
        };
        out.push_str(&format!("  [{tier}] {what}\n"));
    }
    if plan.fixes.len() > 10 {
        out.push_str(&format!("  and {} more\n", plan.fixes.len() - 10));
    }
    if a + b > 0 {
        out.push_str("Run `hq vault health --apply` to fix the certain ones (`--tier all` adds the likely ones).\n");
    }
    if !plan.skipped.is_empty() {
        out.push_str(&skipped_lines(&plan.skipped));
    }
    out
}

/// `hq vault repairs [--json]`: what the check changed, newest first.
pub fn repairs(config: &HqConfig, json: bool) -> Result<()> {
    let rows = match open_db(config) {
        Some(db) => db.with_conn(|c| hq_db::vault_health::repairs(c, 30))?,
        None => Vec::new(),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("Nothing has been changed by the notes check.");
        return Ok(());
    }
    for r in rows {
        let state = if r.undone_at.is_some() {
            "taken back"
        } else {
            "in place"
        };
        println!(
            "{}  {}  {} fix(es), {} note(s) changed, {} removed  ({}, {state})",
            r.run_id, r.ran_at, r.fixes, r.rewritten, r.trashed, r.source
        );
    }
    Ok(())
}

/// `hq vault repair --undo <run>`.
pub fn undo(config: &HqConfig, run: &str) -> Result<()> {
    let Some(db) = open_db(config) else {
        anyhow::bail!("HQ has no database yet");
    };
    let undone =
        hq_tools::vault_health::undo_repair(&config.vault_path, &std::sync::Arc::new(db), run)?;
    println!(
        "Put back {} note(s) and {} removed note(s).",
        undone.restored.len(),
        undone.untrashed.len()
    );
    if !undone.edited_since.is_empty() {
        println!("Left as you have them, since you changed them after the fix:");
        for n in &undone.edited_since {
            println!("  {n}");
        }
    }
    for f in &undone.failed {
        println!("Could not put back: {f}");
    }
    Ok(())
}

/// What the owner chose to ignore, or nothing when there is no database yet.
fn load_ignored(config: &HqConfig) -> Ignored {
    hq_db::Database::open_existing(&config.db_path())
        .ok()
        .and_then(|db| hq_tools::vault_health::load_ignored(&db).ok())
        .unwrap_or_default()
}

/// The report as plain lines for a terminal.
pub fn render_text(r: &Report) -> String {
    let c = &r.counts;
    let mut out = format!("{} notes, {} links\n", c.notes, c.links.total);
    if c.problems() == 0 {
        out.push_str("Nothing needs fixing.\n");
    }
    let mut section = |title: &str, total: usize, lines: Vec<String>| {
        if total == 0 {
            return;
        }
        out.push_str(&format!("\n{title}: {total}\n"));
        for l in &lines {
            out.push_str(&format!("  {l}\n"));
        }
        if total > lines.len() {
            out.push_str(&format!("  and {} more\n", total - lines.len()));
        }
    };
    let shown = |n: usize| n.min(10);
    section(
        "Links that reach no note",
        c.links.dead,
        r.dead_links
            .iter()
            .take(shown(c.links.dead))
            .map(|d| {
                let near = d
                    .did_you_mean
                    .as_deref()
                    .map(|n| format!("  (did you mean {n}?)"))
                    .unwrap_or_default();
                format!("{}:{}  {}{near}", d.note, d.line, d.link)
            })
            .collect(),
    );
    section(
        "Links that fit more than one note",
        c.links.ambiguous,
        r.ambiguous_links
            .iter()
            .take(shown(c.links.ambiguous))
            .map(|a| {
                format!(
                    "{}:{}  {}  ({} notes)",
                    a.note,
                    a.line,
                    a.link,
                    a.candidates.len()
                )
            })
            .collect(),
    );
    section(
        "Links to a heading that is not there",
        c.links.broken_anchors,
        r.broken_anchors
            .iter()
            .take(shown(c.links.broken_anchors))
            .map(|b| format!("{}:{}  {}", b.note, b.line, b.link))
            .collect(),
    );
    section(
        "Notes with the same text",
        c.duplicates,
        r.duplicates
            .iter()
            .take(shown(c.duplicates))
            .map(|g| g.notes.join("  =  "))
            .collect(),
    );
    section(
        "Properties that do not read",
        c.frontmatter,
        r.frontmatter
            .iter()
            .take(shown(c.frontmatter))
            .map(|f| format!("{}  ({:?})", f.note, f.problem))
            .collect(),
    );
    if c.loose > 0 {
        out.push_str(&format!("\nNotes nothing links to: {}\n", c.loose));
    }
    if c.unread > 0 {
        out.push_str(&format!(
            "Notes too large or unreadable, not checked: {}\n",
            c.unread
        ));
    }
    out
}

/// The vault section of `hq doctor`: what the last check found.
pub fn doctor(config: &HqConfig) {
    println!("\nNotes");
    let latest = hq_db::Database::open_existing(&config.db_path())
        .ok()
        .and_then(|db| hq_tools::vault_health::latest(&db).ok().flatten());
    let (label, text) = doctor_line(latest.as_ref(), Utc::now());
    println!("  {label}  {text}");
}

/// The doctor's one line about the notes. Pure over its inputs.
pub fn doctor_line(
    latest: Option<&hq_tools::vault_health::Stored>,
    now: DateTime<Utc>,
) -> (&'static str, String) {
    let Some(stored) = latest else {
        return (
            "--",
            "not checked yet: HQ checks the notes every few hours, or run `hq vault health`".into(),
        );
    };
    let ago = DateTime::parse_from_rfc3339(&stored.ran_at)
        .map(|t| ago(now - t.with_timezone(&Utc)))
        .unwrap_or_else(|_| "at an unknown time".into());
    let c = &stored.report.counts;
    if c.problems() == 0 {
        return (
            "ok",
            format!(
                "{} notes, {} links, nothing needs fixing (checked {ago})",
                c.notes, c.links.total
            ),
        );
    }
    (
        "warn",
        format!(
            "{} to fix: {} links reach no note, {} fit several, {} point at a missing heading, \
             {} duplicates, {} property problems (checked {ago}); run `hq vault health` for the list",
            c.problems(),
            c.links.dead,
            c.links.ambiguous,
            c.links.broken_anchors,
            c.duplicates,
            c.frontmatter
        ),
    )
}

fn ago(d: chrono::Duration) -> String {
    match d.num_minutes() {
        m if m < 2 => "just now".into(),
        m if m < 120 => format!("{m} minutes ago"),
        m if m < 48 * 60 => format!("{} hours ago", m / 60),
        m => format!("{} days ago", m / (24 * 60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::vault_health::Stored;
    use hq_vault::health::{Counts, DeadLink, LinkCounts};

    fn report(dead: usize) -> Report {
        Report {
            counts: Counts {
                notes: 20,
                links: LinkCounts {
                    total: 50,
                    dead,
                    ..LinkCounts::default()
                },
                ..Counts::default()
            },
            dead_links: (0..dead)
                .map(|i| DeadLink {
                    note: "Notebooks/A.md".into(),
                    line: i + 1,
                    link: format!("[[Gone {i}]]"),
                    target: format!("Gone {i}"),
                    did_you_mean: (i == 0).then(|| "Gone Fishing".into()),
                })
                .collect(),
            ..Report::default()
        }
    }

    #[test]
    fn a_clean_report_says_so_and_a_dirty_one_lists_with_a_cap() {
        assert!(render_text(&report(0)).contains("Nothing needs fixing."));
        let text = render_text(&report(12));
        assert!(text.contains("Links that reach no note: 12"), "{text}");
        assert!(
            text.contains("Notebooks/A.md:1  [[Gone 0]]  (did you mean Gone Fishing?)"),
            "{text}"
        );
        assert!(text.contains("and 2 more"), "{text}");
        assert!(!text.contains('\u{2014}'));
    }

    #[test]
    fn the_doctor_line_covers_not_checked_clean_and_dirty() {
        let now = Utc::now();
        assert_eq!(doctor_line(None, now).0, "--");
        let at = |mins: i64| (now - chrono::Duration::minutes(mins)).to_rfc3339();
        let clean = Stored {
            fixes: vec![],
            fix_a: 0,
            fix_b: 0,
            skipped: vec![],
            ran_at: at(1),
            report: report(0),
        };
        let (label, text) = doctor_line(Some(&clean), now);
        assert_eq!(label, "ok");
        assert!(text.contains("just now"), "{text}");
        let dirty = Stored {
            fixes: vec![],
            fix_a: 0,
            fix_b: 0,
            skipped: vec![],
            ran_at: at(300),
            report: report(3),
        };
        let (label, text) = doctor_line(Some(&dirty), now);
        assert_eq!(label, "warn");
        assert!(
            text.contains("3 to fix") && text.contains("5 hours ago"),
            "{text}"
        );
    }
}
