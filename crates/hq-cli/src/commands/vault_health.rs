//! `hq vault health` and the vault section of `hq doctor`.
//!
//! Both read what `hq_tools::vault_health` reads for the web and phone screens, so the same
//! numbers show everywhere. Nothing here changes a note.

use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::config::HqConfig;
use hq_vault::health::{Ignored, Report};

/// `hq vault health [--json]`: check every note now and say what is wrong.
pub fn run(config: &HqConfig, json: bool) -> Result<()> {
    let ignored = load_ignored(config);
    let report = hq_tools::vault_health::scan_now(&config.vault_path, &ignored);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
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
            ran_at: at(1),
            report: report(0),
        };
        let (label, text) = doctor_line(Some(&clean), now);
        assert_eq!(label, "ok");
        assert!(text.contains("just now"), "{text}");
        let dirty = Stored {
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
