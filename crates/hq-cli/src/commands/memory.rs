use anyhow::Result;
use hq_core::config::{HqConfig, MemoryRecall};
use hq_db::memories_owner::{self, ListFilter, OwnerRow, Standing};
use hq_vault::VaultClient;

/// Show memory facts, edit memory, and manage system context.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    let vault = VaultClient::new(config.vault_path.clone())?;

    match sub {
        "show" | "" => show(config, &vault)?,
        "facts" => facts(&vault)?,
        "add" => add(&vault, args)?,
        "soul" => soul(&vault)?,
        "preferences" | "prefs" => preferences(&vault)?,
        "context" => context(&vault)?,
        "graph" => graph(config, args)?,
        "stats" => stats(config)?,
        "recall-report" | "recall" => recall_report(config, args)?,
        "list" => list_cmd(config, args)?,
        "search" => search_cmd(config, args)?,
        "view" => view_cmd(config, args)?,
        "edit" => edit_cmd(config, args)?,
        "pin" => pin_cmd(config, args, true)?,
        "unpin" => pin_cmd(config, args, false)?,
        "keep" => standing_cmd(config, args, Standing::Active)?,
        "outdated" => standing_cmd(config, args, Standing::OutOfDate)?,
        "forget" => forget_cmd(config, args)?,
        "export" => export_cmd(config)?,
        _ => usage()?,
    }

    Ok(())
}

/// Prints the whole of MEMORY.md.
fn show(config: &HqConfig, vault: &VaultClient) -> Result<()> {
    let memory = vault.read_system_file("MEMORY.md")?;
    if memory.is_empty() {
        println!("No memory stored yet.");
        println!(
            "Memory is written to: {}",
            config.vault_path.join("_system/MEMORY.md").display()
        );
    } else {
        println!("{}", memory);
    }
    Ok(())
}

/// Lists the bullet lines of MEMORY.md under their headings.
fn facts(vault: &VaultClient) -> Result<()> {
    let memory = vault.read_system_file("MEMORY.md")?;
    if memory.is_empty() {
        println!("No memory stored yet.");
        return Ok(());
    }

    // Extract lines that look like facts (bullet points under headings)
    println!("Memory Facts");
    println!("============\n");

    let mut in_section = false;
    let mut section_name;

    for line in memory.lines() {
        if let Some(stripped) = line.strip_prefix("## ") {
            section_name = stripped.trim().to_string();
            in_section = true;
            println!("\n{}:", section_name);
        } else if line.starts_with("# ") {
            in_section = false;
        } else if in_section && (line.starts_with("- ") || line.starts_with("* ")) {
            println!("  {}", line);
        }
    }
    Ok(())
}

/// Appends a fact to MEMORY.md.
fn add(vault: &VaultClient, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Usage: hq memory add \"<fact>\"");
    }
    let fact = args.join(" ");
    let mut memory = vault.read_system_file("MEMORY.md")?;

    if memory.is_empty() {
        memory = "---\nnoteType: system-file\nfileName: memory\nversion: 1\npinned: true\n---\n# Agent Memory\n\n## Key Facts\n\n".to_string();
    }

    // Add the fact under Key Facts section
    if let Some(pos) = memory.find("## Key Facts") {
        let insert_pos = memory[pos..]
            .find("\n\n")
            .map(|i| pos + i)
            .unwrap_or(memory.len());
        let new_line = format!("\n- {}", fact);
        memory.insert_str(insert_pos, &new_line);
    } else {
        memory.push_str(&format!("\n- {}\n", fact));
    }

    vault.write_system_file("MEMORY.md", &memory)?;
    println!("Added to memory: {}", fact);
    Ok(())
}

/// Prints SOUL.md.
fn soul(vault: &VaultClient) -> Result<()> {
    let soul = vault.read_system_file("SOUL.md")?;
    if soul.is_empty() {
        println!("No soul file yet. Run `hq setup` to create one.");
    } else {
        println!("{}", soul);
    }
    Ok(())
}

/// Prints PREFERENCES.md.
fn preferences(vault: &VaultClient) -> Result<()> {
    let prefs = vault.read_system_file("PREFERENCES.md")?;
    if prefs.is_empty() {
        println!("No preferences set yet.");
    } else {
        println!("{}", prefs);
    }
    Ok(())
}

/// Summarises the system context HQ loads into a session.
fn context(vault: &VaultClient) -> Result<()> {
    let ctx = vault.get_system_context()?;
    println!("System Context Summary");
    println!("======================\n");
    println!("SOUL:        {} chars", ctx.soul.len());
    println!("MEMORY:      {} chars", ctx.memory.len());
    println!("PREFERENCES: {} chars", ctx.preferences.len());
    println!("HEARTBEAT:   {} chars", ctx.heartbeat.len());
    println!("Config keys: {}", ctx.config.len());
    println!("Pinned notes: {}", ctx.pinned_notes.len());

    if !ctx.pinned_notes.is_empty() {
        println!("\nPinned:");
        for note in &ctx.pinned_notes {
            println!("  - {} ({})", note.title, note.path);
        }
    }

    println!(
        "\nPinned-note directories scanned: {}",
        ctx.pinned_scan.scanned.join(", ")
    );
    if !ctx.pinned_scan.skipped.is_empty() {
        println!("Skipped (missing): {}", ctx.pinned_scan.skipped.join(", "));
    }
    Ok(())
}

/// Explores the entities connected to a seed entity.
fn graph(config: &HqConfig, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Usage: hq memory graph <seed_entity> [hops] [limit]");
    }
    let seed = &args[0];
    let hops = args
        .get(1)
        .and_then(|h| h.parse::<usize>().ok())
        .unwrap_or(2);
    let limit = args
        .get(2)
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(10);

    // The entity graph lives in the shared vault database, like every other
    // memory command and the `vault_graph` tool.
    let db = hq_db::Database::open(&config.db_path())?;
    let results = hq_memory::entity_graph::spreading_activation(
        &db,
        std::slice::from_ref(seed),
        hops,
        0.1,
        limit,
    )?;

    println!(
        "Entity Graph: Spreading Activation from '{}' ({} hops)",
        seed, hops
    );
    println!("====================================================\n");

    if results.is_empty() {
        println!("No related entities found.");
    } else {
        for ent in results {
            println!(
                "{:<20} {:<10} activation: {:.2}",
                ent.display_name,
                format!("[{}]", ent.entity_type),
                ent.activation
            );
        }
    }
    Ok(())
}

// ─── What HQ learned from chats ─────────────────────────────────────────────

fn open_db(config: &HqConfig) -> Result<hq_db::Database> {
    hq_db::Database::open(&config.db_path())
}

/// The memory number in `args`, accepting `12` and `#12`.
fn number(args: &[String]) -> Result<i64> {
    args.first()
        .and_then(|a| a.trim_start_matches('#').parse().ok())
        .ok_or_else(|| anyhow::anyhow!("Give the memory's number, as shown by `hq memory list`."))
}

fn missing(id: i64) -> anyhow::Error {
    anyhow::anyhow!("There is no memory #{id}.")
}

/// How long ago, in the units a person says it in.
fn ago(created_at: &str) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return "some time ago".to_string();
    };
    let days = (chrono::Utc::now() - at.with_timezone(&chrono::Utc))
        .num_days()
        .max(0);
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

/// Words for what is unusual about a memory; nothing for an ordinary one.
fn notes_on(row: &OwnerRow) -> Vec<&'static str> {
    let mut notes = Vec::new();
    if row.pinned {
        notes.push("pinned");
    }
    match row.status.as_str() {
        "review" => notes.push("held back"),
        "superseded" => notes.push("out of date"),
        _ => {}
    }
    if row.origin != "owner" {
        notes.push("from a shared channel, not yet kept");
    }
    if row.provenance == "owner_edited" {
        notes.push("edited by you");
    }
    notes
}

/// One line for a list.
fn render_line(row: &OwnerRow) -> String {
    let notes = notes_on(row);
    let tag = if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    };
    let text: String = row.summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.chars().count() > 110 {
        format!("{}...", text.chars().take(110).collect::<String>())
    } else {
        text
    };
    format!("#{:<5} {:<14} {text}{tag}", row.id, ago(&row.created_at))
}

fn print_rows(rows: &[OwnerRow], none: &str) {
    if rows.is_empty() {
        println!("{none}");
    }
    for row in rows {
        println!("{}", render_line(row));
    }
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn list_cmd(config: &HqConfig, args: &[String]) -> Result<()> {
    let db = open_db(config)?;
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(30);
    let status = if flag(args, "--review") {
        Some("review".to_string())
    } else if flag(args, "--outdated") {
        Some("superseded".to_string())
    } else {
        None
    };
    let (rows, counts) = db.with_conn(|conn| {
        let filter = ListFilter {
            status,
            unattributed_only: flag(args, "--unattributed"),
            pinned_only: flag(args, "--pinned"),
            limit,
            offset: 0,
        };
        Ok((
            memories_owner::list(conn, &filter)?,
            memories_owner::counts(conn)?,
        ))
    })?;
    println!(
        "{} memories: {} in use, {} held back, {} out of date, {} from shared channels, {} pinned\n",
        counts.total,
        counts.active,
        counts.review,
        counts.out_of_date,
        counts.unattributed,
        counts.pinned
    );
    print_rows(&rows, "Nothing matches.");
    Ok(())
}

fn search_cmd(config: &HqConfig, args: &[String]) -> Result<()> {
    if args.is_empty() {
        anyhow::bail!("Give the words to look for: hq memory search acme delivery");
    }
    let rows =
        open_db(config)?.with_conn(|conn| memories_owner::search(conn, &args.join(" "), 20))?;
    print_rows(&rows, "No memory matches.");
    Ok(())
}

fn view_cmd(config: &HqConfig, args: &[String]) -> Result<()> {
    let id = number(args)?;
    let row = open_db(config)?
        .with_conn(|conn| memories_owner::get(conn, id))?
        .ok_or_else(|| missing(id))?;
    println!("Memory #{}", row.id);
    println!("  {}", row.summary);
    println!("  learned:   {} ({})", ago(&row.created_at), row.harness);
    println!("  status:    {}", row.status);
    println!("  from:      {}", row.origin);
    println!("  how:       {}", row.provenance);
    println!("  pinned:    {}", if row.pinned { "yes" } else { "no" });
    println!("  brought up in a chat: {} times", row.used);
    if !row.entities.is_empty() {
        println!("  names:     {}", row.entities.join(", "));
    }
    if !row.topics.is_empty() {
        println!("  topics:    {}", row.topics.join(", "));
    }
    if let Some(thread) = &row.thread_id {
        println!("  chat:      {thread}");
    }
    Ok(())
}

fn edit_cmd(config: &HqConfig, args: &[String]) -> Result<()> {
    let id = number(args)?;
    let text = args[1..].join(" ");
    if text.trim().is_empty() {
        anyhow::bail!("Give the corrected text: hq memory edit {id} \"the new text\"");
    }
    let changed = open_db(config)?.with_conn(|conn| memories_owner::edit(conn, id, &text))?;
    if !changed {
        return Err(missing(id));
    }
    println!("Memory #{id} now says what you wrote. It counts as yours.");
    Ok(())
}

fn pin_cmd(config: &HqConfig, args: &[String], pinned: bool) -> Result<()> {
    let id = number(args)?;
    if !open_db(config)?.with_conn(|conn| memories_owner::set_pinned(conn, id, pinned))? {
        return Err(missing(id));
    }
    println!(
        "Memory #{id} {}.",
        if pinned {
            "is pinned and will not age out"
        } else {
            "can age out again"
        }
    );
    Ok(())
}

fn standing_cmd(config: &HqConfig, args: &[String], standing: Standing) -> Result<()> {
    let id = number(args)?;
    if !open_db(config)?.with_conn(|conn| memories_owner::set_standing(conn, id, standing))? {
        return Err(missing(id));
    }
    match standing {
        Standing::OutOfDate => {
            println!("Memory #{id} is kept but marked out of date. It will not be brought up.")
        }
        _ => println!("Memory #{id} is yours and can be brought up when it fits."),
    }
    Ok(())
}

fn forget_cmd(config: &HqConfig, args: &[String]) -> Result<()> {
    let id = number(args)?;
    let db = open_db(config)?;
    let row = db
        .with_conn(|conn| memories_owner::get(conn, id))?
        .ok_or_else(|| missing(id))?;
    if !flag(args, "--yes") {
        println!("This would delete memory #{id}, and the notes and lines HQ made from it:\n");
        println!("  {}\n", render_line(&row));
        println!("It cannot be undone. Run again with --yes to do it:");
        println!("  hq memory forget {id} --yes");
        return Ok(());
    }
    let Some(report) = hq_memory::forget::forget(&db, &config.vault_path, id)? else {
        return Err(missing(id));
    };
    println!("Forgot memory #{id}.");
    if report.insights_removed > 0 {
        println!("  insights removed: {}", report.insights_removed);
    }
    for note in &report.insight_notes_removed {
        println!("  note deleted: {note}");
    }
    let pages = report.concept_pages_updated + report.concept_pages_removed;
    if pages > 0 {
        println!(
            "  concept pages: {} updated, {} removed",
            report.concept_pages_updated, report.concept_pages_removed
        );
    }
    if report.archive_entries_removed > 0 {
        println!(
            "  archive lines removed: {}",
            report.archive_entries_removed
        );
    }
    println!("\nHQ will not learn those exact words again. What it could not reach:");
    for what in hq_memory::forget::NOT_REACHED {
        println!("  - {what}");
    }
    Ok(())
}

fn export_cmd(config: &HqConfig) -> Result<()> {
    let all = open_db(config)?.with_conn(|conn| {
        // One page is capped, so read page after page until there are no more.
        let mut all: Vec<OwnerRow> = Vec::new();
        loop {
            let page = memories_owner::list(
                conn,
                &ListFilter {
                    limit: memories_owner::MAX_PAGE,
                    offset: all.len() as i64,
                    ..ListFilter::default()
                },
            )?;
            if page.is_empty() {
                return Ok(all);
            }
            all.extend(page);
        }
    })?;
    println!("{}", serde_json::to_string_pretty(&all)?);
    Ok(())
}

/// How recall has done over the last few days, and whether it is ready to be switched on.
fn recall_report(config: &HqConfig, args: &[String]) -> Result<()> {
    let days: i64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(7);
    let db = hq_db::Database::open(&config.db_path())?;
    let report = db.with_conn(|conn| hq_db::memory_recall_log::report(conn, days))?;
    print!(
        "{}",
        render_recall_report(&report, config.daemon.memory_recall)
    );
    Ok(())
}

/// The shortest run of turns worth judging recall by.
const MIN_TURNS_TO_JUDGE: i64 = 50;
/// The slowest a lookup may be at the slow end, in milliseconds, before a chat would feel it.
const SLOW_MS: u64 = 150;
/// The most tokens one block may take.
const BLOCK_TOKENS: u64 = 500;

fn render_recall_report(r: &hq_db::memory_recall_log::Report, mode: MemoryRecall) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let title = format!("Memory recall, last {} days", r.days);
    let _ = writeln!(out, "{title}\n{}\n", "=".repeat(title.len()));
    let mode_note = match mode {
        MemoryRecall::Shadow => {
            "shadow: chats still get the most important memories. What would fit is only written down."
        }
        MemoryRecall::Query => "query: each message brings up only the memories that fit it.",
        MemoryRecall::Recent => {
            "recent: chats get the most important memories. Nothing is worked out."
        }
        MemoryRecall::Off => "off: memories HQ learned from chats are not brought up at all.",
    };
    let _ = writeln!(out, "Mode: {mode_note}");
    if r.turns == 0 {
        let _ = writeln!(
            out,
            "\nNothing has been written down yet. It is recorded while the mode is shadow or query."
        );
        return out;
    }
    let share = |n: i64| 100.0 * n as f64 / r.turns as f64;
    let _ = writeln!(out, "\nMessages looked at: {}", r.turns);
    let _ = writeln!(
        out,
        "  brought up something: {} ({:.0}%), {:.1} memories a message on average",
        r.with_memories,
        share(r.with_memories),
        r.mean_items
    );
    let _ = writeln!(out, "  said nothing: {}", r.turns - r.with_memories);
    for (reason, n) in &r.silent {
        let _ = writeln!(out, "    {reason}: {n}");
    }
    let _ = writeln!(
        out,
        "\nSpeed: half took {} ms or less, 95% took {} ms or less, the slowest {} ms",
        r.latency_ms.p50, r.latency_ms.p95, r.latency_ms.max
    );
    let _ = writeln!(
        out,
        "Size when something was brought up: typically {} tokens, 95% under {}, the largest {}",
        r.tokens.p50, r.tokens.p95, r.tokens.max
    );

    let _ = writeln!(out, "\nReady to switch to query mode?");
    let mut check = |ok: bool, what: String| {
        let _ = writeln!(out, "  {}: {what}", if ok { "yes" } else { "not yet" });
    };
    check(
        r.turns >= MIN_TURNS_TO_JUDGE,
        format!(
            "at least {MIN_TURNS_TO_JUDGE} messages looked at ({})",
            r.turns
        ),
    );
    check(
        r.failures == 0,
        format!("no failures or time-outs ({})", r.failures),
    );
    check(
        r.latency_ms.p95 < SLOW_MS,
        format!("slow end under {SLOW_MS} ms ({} ms)", r.latency_ms.p95),
    );
    check(
        r.tokens.p95 <= BLOCK_TOKENS,
        format!("blocks under {BLOCK_TOKENS} tokens ({})", r.tokens.p95),
    );
    let _ = writeln!(
        out,
        "To switch: set daemon.memory_recall to query in ~/.hq/config.yaml, or HQ_DAEMON__MEMORY_RECALL=query."
    );
    out
}

/// Memory database counts and the activity dashboard.
fn stats(config: &HqConfig) -> Result<()> {
    let db_path = config.db_path();
    let db = hq_db::Database::open(&db_path)?;

    let stats = hq_memory::db::get_memory_stats(&db)?;

    println!("Memory System Stats");
    println!("===================\n");
    println!(
        "  Memories:       {} total ({} unconsolidated)",
        stats.total, stats.unconsolidated
    );
    println!("  Consolidations: {}", stats.consolidations);

    // Entity stats
    let entity_count: i64 = db
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM entity_nodes", [], |r| r.get(0))
                .map_err(Into::into)
        })
        .unwrap_or(0);
    let edge_count: i64 = db
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM entity_edges", [], |r| r.get(0))
                .map_err(Into::into)
        })
        .unwrap_or(0);
    println!(
        "  Entities:       {} nodes, {} edges",
        entity_count, edge_count
    );

    // Recent activity (last 24h)
    println!("\n  Recent Activity (24h)");
    println!("  ---------------------");
    // `created_at` holds an RFC 3339 string, so the cutoff must be one too: comparing it with
    // `unixepoch()` made every row count as new.
    let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let recent_memories: i64 = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE created_at > ?1",
                [&since],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })
        .unwrap_or(0);
    println!("  New memories:   {}", recent_memories);

    // Source breakdown
    println!("\n  Source Breakdown");
    println!("  ----------------");
    let sources: Vec<(String, i64)> = db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT source, COUNT(*) as cnt FROM memories GROUP BY source ORDER BY cnt DESC LIMIT 10"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }).unwrap_or_default();
    for (source, count) in &sources {
        println!("  {:<20} {}", source, count);
    }

    // Consolidation lock status
    let lock_path = config.vault_path.join("_data/.consolidate-lock");
    if lock_path.exists()
        && let Ok(meta) = std::fs::metadata(&lock_path)
        && let Ok(modified) = meta.modified()
    {
        let elapsed = modified.elapsed().unwrap_or_default();
        let hours = elapsed.as_secs() / 3600;
        let mins = (elapsed.as_secs() % 3600) / 60;
        println!("\n  Last consolidation: {}h {}m ago", hours, mins);
    }

    // Dream timestamp
    let dream_path = config.vault_path.join("_system/.last-dream");
    if dream_path.exists()
        && let Ok(meta) = std::fs::metadata(&dream_path)
        && let Ok(modified) = meta.modified()
    {
        let elapsed = modified.elapsed().unwrap_or_default();
        let hours = elapsed.as_secs() / 3600;
        let mins = (elapsed.as_secs() % 3600) / 60;
        println!("  Last dream cycle:   {}h {}m ago", hours, mins);
    }
    Ok(())
}

/// Lists the subcommands.
fn usage() -> Result<()> {
    println!("Usage: hq memory <subcommand>");
    println!();
    println!("Subcommands:");
    println!("  show             Show full MEMORY.md");
    println!("  facts            Extract memory facts");
    println!("  add \"<fact>\"     Add a fact to memory");
    println!("  soul             Show SOUL.md");
    println!("  preferences      Show PREFERENCES.md");
    println!("  context          Show system context summary");
    println!("  graph <entity>   Explore entity connections");
    println!("  stats            Memory DB stats + activity dashboard");
    println!("  recall-report [days]  How memory recall has done lately (default 7 days)");
    println!();
    println!("What HQ learned from your chats (each has a number):");
    println!("  list [--review] [--unattributed] [--pinned] [--outdated] [--limit N]");
    println!("  search <words>   Find a memory");
    println!("  view <number>    Everything HQ knows about one memory");
    println!("  edit <number> \"<text>\"   Correct it. It becomes yours");
    println!("  pin <number> / unpin <number>   Never / again let it age out");
    println!("  keep <number>    Vouch for one held back (from a shared channel)");
    println!("  outdated <number>  It is no longer true: keep it, never bring it up");
    println!("  forget <number> --yes   Delete it and everything HQ made from it");
    println!("  export           Everything, as JSON");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::memory_recall_log::{Percentiles, Report};

    fn report(turns: i64, failures: i64, p95: u64) -> Report {
        Report {
            days: 7,
            turns,
            with_memories: turns / 3,
            silent: vec![("nothing fits".to_string(), turns - turns / 3)],
            failures,
            mean_items: 0.5,
            latency_ms: Percentiles {
                p50: 3,
                p95,
                max: p95 + 5,
            },
            tokens: Percentiles {
                p50: 40,
                p95: 90,
                max: 120,
            },
        }
    }

    #[test]
    fn an_empty_log_says_so_instead_of_printing_zeros() {
        let text = render_recall_report(&report(0, 0, 0), MemoryRecall::Shadow);
        assert!(text.contains("Nothing has been written down yet"), "{text}");
        assert!(!text.contains("Ready to switch"));
    }

    #[test]
    fn a_healthy_log_is_ready_and_says_how_to_switch() {
        let text = render_recall_report(&report(120, 0, 9), MemoryRecall::Shadow);
        assert!(text.contains("Messages looked at: 120"), "{text}");
        assert!(text.contains("nothing fits: 80"), "{text}");
        assert_eq!(text.matches("  yes:").count(), 4, "{text}");
        assert!(!text.contains("not yet"), "{text}");
        assert!(text.contains("HQ_DAEMON__MEMORY_RECALL=query"));
    }

    #[test]
    fn too_little_data_a_failure_or_a_slow_lookup_is_not_ready() {
        let few = render_recall_report(&report(10, 0, 9), MemoryRecall::Shadow);
        assert!(few.contains("not yet: at least 50 messages"), "{few}");
        let failed = render_recall_report(&report(120, 2, 9), MemoryRecall::Shadow);
        assert!(
            failed.contains("not yet: no failures or time-outs (2)"),
            "{failed}"
        );
        let slow = render_recall_report(&report(120, 0, 400), MemoryRecall::Shadow);
        assert!(
            slow.contains("not yet: slow end under 150 ms (400 ms)"),
            "{slow}"
        );
    }

    #[test]
    fn each_mode_is_named_in_plain_words() {
        for (mode, word) in [
            (MemoryRecall::Shadow, "shadow"),
            (MemoryRecall::Query, "query"),
            (MemoryRecall::Recent, "recent"),
            (MemoryRecall::Off, "off"),
        ] {
            let text = render_recall_report(&report(0, 0, 0), mode);
            assert!(text.contains(&format!("Mode: {word}:")), "{text}");
        }
    }

    fn row(summary: &str) -> OwnerRow {
        OwnerRow {
            id: 12,
            summary: summary.to_string(),
            entities: vec![],
            topics: vec![],
            importance: 0.5,
            created_at: (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339(),
            status: "active".into(),
            origin: "owner".into(),
            provenance: "model_inferred".into(),
            pinned: false,
            used: 0,
            thread_id: None,
            harness: "web".into(),
        }
    }

    #[test]
    fn a_memory_number_is_read_with_or_without_a_hash() {
        assert_eq!(number(&["12".to_string()]).unwrap(), 12);
        assert_eq!(number(&["#12".to_string()]).unwrap(), 12);
        assert!(number(&[]).is_err());
        assert!(number(&["acme".to_string()]).is_err());
    }

    #[test]
    fn a_list_line_has_the_number_the_age_and_what_is_unusual_about_it() {
        let plain = render_line(&row("Acme delivers on Thursdays"));
        assert!(plain.starts_with("#12"), "{plain}");
        assert!(
            plain.contains("3 days ago") && plain.ends_with("Acme delivers on Thursdays"),
            "{plain}"
        );

        let mut odd = row("Group chat moves to Sundays");
        odd.pinned = true;
        odd.status = "review".into();
        odd.origin = "unknown_channel".into();
        let line = render_line(&odd);
        assert!(
            line.contains("pinned") && line.contains("held back"),
            "{line}"
        );
        assert!(
            line.contains("from a shared channel, not yet kept"),
            "{line}"
        );
    }

    #[test]
    fn a_long_memory_is_cut_in_a_list_and_a_line_break_cannot_split_it() {
        let long = format!("start\nsecond line {}", "word ".repeat(60));
        let line = render_line(&row(&long));
        assert!(!line.contains('\n'), "{line:?}");
        assert!(line.ends_with("..."), "{line}");
    }

    #[test]
    fn ages_are_said_the_way_a_person_says_them() {
        let days = |n| ago(&(chrono::Utc::now() - chrono::Duration::days(n)).to_rfc3339());
        assert_eq!(days(0), "today");
        assert_eq!(days(1), "yesterday");
        assert_eq!(days(5), "5 days ago");
        assert_eq!(days(30), "4 weeks ago");
        assert_eq!(days(200), "6 months ago");
        assert_eq!(days(800), "2 years ago");
        assert_eq!(ago("not a date"), "some time ago");
    }
}
