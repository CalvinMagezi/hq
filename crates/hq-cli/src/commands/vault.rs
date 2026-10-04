use anyhow::Result;
use hq_core::config::HqConfig;
use hq_vault::VaultClient;

/// Vault operations: list, read, write, search.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    let vault = VaultClient::new(config.vault_path.clone())?;

    match sub {
        "list" | "ls" => {
            let dir = args.first().map(|s| s.as_str()).unwrap_or("Notebooks");
            let notes = vault.list_notes(dir)?;
            if notes.is_empty() {
                println!("No notes in {}/", dir);
            } else {
                println!("Notes in {}/ ({}):", dir, notes.len());
                for note in &notes {
                    println!("  {}", note);
                }
            }
        }
        "tree" => {
            let dir = args.first().map(|s| s.as_str()).unwrap_or("");
            let notes = vault.list_notes_recursive(dir)?;
            println!("Vault notes ({} total):", notes.len());
            for note in &notes {
                println!("  {}", note);
            }
        }
        "read" | "cat" => {
            let path = args
                .first()
                .ok_or_else(|| anyhow::anyhow!("Usage: hq vault read <path>"))?;
            let note = vault.read_note(path)?;
            println!("# {}\n", note.title);
            if !note.tags.is_empty() {
                println!("Tags: {}", note.tags.join(", "));
            }
            if note.pinned {
                println!("Pinned: true");
            }
            println!();
            println!("{}", note.content);
        }
        "write" => {
            if args.len() < 2 {
                anyhow::bail!("Usage: hq vault write <path> <content>");
            }
            let path = &args[0];
            let content = args[1..].join(" ");

            let note = hq_core::types::Note {
                title: std::path::PathBuf::from(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone()),
                content,
                path: path.clone(),
                frontmatter: std::collections::HashMap::new(),
                note_type: None,
                tags: Vec::new(),
                pinned: false,
                source: None,
                embedding_status: None,
                created_at: None,
                updated_at: None,
                modified_at: chrono::Utc::now(),
            };

            vault.write_note(path, &note)?;
            println!("Wrote: {}", path);
        }
        "stats" => {
            let (note_count, db_size) = vault.get_stats()?;
            println!("Vault Statistics");
            println!("================");
            println!("Path:     {}", config.vault_path.display());
            println!("Notes:    {}", note_count);
            println!("DB size:  {:.1} MB", db_size as f64 / 1_048_576.0);
        }
        "context" => {
            let ctx = vault.get_system_context()?;
            println!("System Context");
            println!("==============");
            println!();
            if !ctx.soul.is_empty() {
                println!("SOUL ({} chars)", ctx.soul.len());
            } else {
                println!("SOUL: (not set)");
            }
            if !ctx.memory.is_empty() {
                println!("MEMORY ({} chars)", ctx.memory.len());
            } else {
                println!("MEMORY: (not set)");
            }
            if !ctx.preferences.is_empty() {
                println!("PREFERENCES ({} chars)", ctx.preferences.len());
            } else {
                println!("PREFERENCES: (not set)");
            }
            if !ctx.config.is_empty() {
                println!("\nConfig:");
                for (k, v) in &ctx.config {
                    println!("  {}: {}", k, v);
                }
            }
            println!("\nPinned notes: {}", ctx.pinned_notes.len());
            for note in &ctx.pinned_notes {
                println!("  - {} ({})", note.title, note.path);
            }
        }
        _ => {
            println!("Usage: hq vault <subcommand>");
            println!();
            println!("Subcommands:");
            println!("  list [dir]           List notes in a directory");
            println!("  tree [dir]           List all notes recursively");
            println!("  read <path>          Read a note");
            println!("  write <path> <text>  Write a note");
            println!("  stats                Show vault statistics");
            println!("  context              Show system context");
        }
    }

    Ok(())
}
