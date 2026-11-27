use anyhow::Result;
use hq_core::config::{HqConfig, MemoryRecall};
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
}
