use anyhow::Result;
use hq_core::config::HqConfig;
use hq_vault::VaultClient;

/// Show memory facts, edit memory, and manage system context.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    let vault = VaultClient::new(config.vault_path.clone())?;

    match sub {
        "show" | "" => {
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
        }
        "facts" => {
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
        }
        "add" => {
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
        }
        "soul" => {
            let soul = vault.read_system_file("SOUL.md")?;
            if soul.is_empty() {
                println!("No soul file yet. Run `hq setup` to create one.");
            } else {
                println!("{}", soul);
            }
        }
        "preferences" | "prefs" => {
            let prefs = vault.read_system_file("PREFERENCES.md")?;
            if prefs.is_empty() {
                println!("No preferences set yet.");
            } else {
                println!("{}", prefs);
            }
        }
        "context" => {
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
                println!(
                    "Skipped (missing): {}",
                    ctx.pinned_scan.skipped.join(", ")
                );
            }
        }
        "graph" => {
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

            let db_path = config.vault_path.join("_embeddings/hq.db");
            let db = hq_db::Database::open(&db_path)?;
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
        }
        "stats" => {
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
            let recent_memories: i64 = db
                .with_conn(|conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM memories WHERE created_at > unixepoch() - 86400",
                        [],
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
        }
        _ => {
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
        }
    }

    Ok(())
}
