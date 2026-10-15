use anyhow::Result;
use hq_core::config::HqConfig;
use hq_vault::VaultClient;

/// Vault operations: list, read, write, search.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    let vault = VaultClient::new(config.vault_path.clone())?;
    let lite = config.profile.is_lite();
    // `--json` on list, tree and read prints one JSON value instead of text.
    // Only for the read-only listings: in `write` and `export` the word is the user's own text.
    let json_verb = matches!(sub, "list" | "ls" | "tree" | "read" | "cat");
    let json = json_verb && args.iter().any(|a| a == "--json");
    let args: Vec<String> = args
        .iter()
        .filter(|a| !(json_verb && *a == "--json"))
        .cloned()
        .collect();
    let args = &args[..];

    // Under Lite the terminal sees what the web app sees: HQ's own folders (`_system`, `_data`,
    // `.git`) are not notes, and a symlink cannot lead into them.
    let deny = |rel: &str| -> Result<()> {
        if lite
            && (hq_web::lite_hides(rel)
                || hq_web::lite_hides_resolved(
                    &config.vault_path,
                    &config.vault_path.join(rel.trim()),
                ))
        {
            anyhow::bail!("`{rel}` is not part of the notes in HQ Lite (profile: lite)");
        }
        Ok(())
    };
    // A note listed by name may sit behind a symlink into a hidden folder, so Lite also checks
    // where each one really is.
    let visible = |n: &String| {
        !lite
            || !(hq_web::lite_hides(n)
                || hq_web::lite_hides_resolved(&config.vault_path, &config.vault_path.join(n)))
    };

    match sub {
        "list" | "ls" => {
            let dir = args.first().map(|s| s.as_str()).unwrap_or("Notebooks");
            deny(dir)?;
            let notes: Vec<String> = vault.list_notes(dir)?.into_iter().filter(visible).collect();
            if json {
                println!("{}", serde_json::json!({ "dir": dir, "notes": notes }));
            } else if notes.is_empty() {
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
            if !dir.trim().is_empty() {
                deny(dir)?;
            }
            let notes: Vec<String> = vault
                .list_notes_recursive(dir)?
                .into_iter()
                .filter(visible)
                .collect();
            if json {
                println!("{}", serde_json::json!({ "dir": dir, "notes": notes }));
            } else {
                println!("Vault notes ({} total):", notes.len());
                for note in &notes {
                    println!("  {}", note);
                }
            }
        }
        "read" | "cat" => {
            let path = args
                .first()
                .ok_or_else(|| anyhow::anyhow!("Usage: hq vault read <path>"))?;
            deny(path)?;
            let note = vault.read_note(path)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "path": path,
                        "title": note.title,
                        "tags": note.tags,
                        "pinned": note.pinned,
                        "content": note.content,
                    })
                );
                return Ok(());
            }
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
            deny(path)?;
            // A lone `-` reads the note text from standard input.
            let content = if args.len() == 2 && args[1] == "-" {
                use std::io::Read;
                let mut s = String::new();
                const MAX: u64 = 8 * 1024 * 1024;
                std::io::stdin().take(MAX + 1).read_to_string(&mut s)?;
                if s.len() as u64 > MAX {
                    anyhow::bail!("the note text on standard input is over 8 MiB");
                }
                s
            } else {
                args[1..].join(" ")
            };

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
            record_owner_write(config, path);
            println!("Wrote: {}", path);
        }
        "export" | "export-pdf" | "pdf" => {
            let pdf_only = sub != "export";
            let usage = if pdf_only {
                "Usage: hq vault export-pdf <note> [-o <file.pdf>] [--brand <slug>]"
            } else {
                "Usage: hq vault export <note> --format <fmt> [-o <file>] [--brand <slug>] [--lang <language>]..."
            };
            let mut note_ref: Option<&str> = None;
            let mut output: Option<&str> = None;
            let mut brand: Option<&str> = None;
            let mut format: Option<&str> = None;
            let mut languages: Vec<String> = Vec::new();
            let mut it = args.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "-o" | "--output" => {
                        output = Some(it.next().ok_or_else(|| anyhow::anyhow!(usage))?)
                    }
                    "--brand" => brand = Some(it.next().ok_or_else(|| anyhow::anyhow!(usage))?),
                    "-f" | "--format" if !pdf_only => {
                        format = Some(it.next().ok_or_else(|| anyhow::anyhow!(usage))?)
                    }
                    "--lang" | "--language" if !pdf_only => {
                        languages.push(it.next().ok_or_else(|| anyhow::anyhow!(usage))?.clone())
                    }
                    other if other.starts_with('-') => {
                        anyhow::bail!("unknown option {other}\n{usage}")
                    }
                    other if note_ref.is_none() => note_ref = Some(other),
                    _ => anyhow::bail!(usage),
                }
            }
            let note_ref = note_ref.ok_or_else(|| anyhow::anyhow!(usage))?;
            deny(note_ref)?;
            let format = if pdf_only {
                hq_export::Format::Pdf
            } else {
                let raw = format.ok_or_else(|| anyhow::anyhow!("--format is required\n{usage}"))?;
                raw.parse::<hq_export::Format>()
                    .map_err(|e| anyhow::anyhow!("{e}"))?
            };
            let note = hq_convert::note_pdf::resolve_note(&config.vault_path, note_ref)
                .ok_or_else(|| anyhow::anyhow!("note not found in the vault: {note_ref}"))?;
            if lite && hq_web::lite_hides_resolved(&config.vault_path, &note) {
                anyhow::bail!("`{note_ref}` is not part of the notes in HQ Lite (profile: lite)");
            }
            let kit = brand
                .map(|slug| hq_convert::brand::load_brand_kit(&config.vault_path, slug))
                .transpose()
                .map_err(|e| anyhow::anyhow!("brand resolution failed: {e}"))?;
            let done =
                hq_export::export_note(&config.vault_path, &note, format, kit.as_ref(), &languages)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            // The extension comes from the result: several tables export as a zip.
            let dest = match output {
                Some(o) => std::path::PathBuf::from(o),
                None => {
                    let stem = note.file_stem().map(|s| s.to_string_lossy().into_owned());
                    std::path::PathBuf::from(format!(
                        "{}.{}",
                        stem.as_deref().unwrap_or("note"),
                        done.output.extension
                    ))
                }
            };
            if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &done.output.bytes)?;
            println!(
                "Wrote {} ({:.1} KB, {})",
                dest.display(),
                done.output.bytes.len() as f64 / 1024.0,
                format
            );
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
            if lite {
                anyhow::bail!(
                    "`vault context` reads HQ's own notes, which are not part of HQ Lite"
                );
            }
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
        "repair" => {
            let apply = args.iter().any(|a| a == "--apply");
            let db = hq_db::Database::open_existing(&config.db_path())?;
            let vault_path = config.vault_path.clone();
            let report =
                db.with_conn(move |conn| hq_db::search::repair_orphans(conn, &vault_path, apply))?;
            print!("{}", repair_summary(&report, apply));
        }
        _ => {
            println!("Usage: hq vault <subcommand>");
            println!();
            println!("Subcommands:");
            println!("  list [dir]           List notes in a directory");
            println!("  tree [dir]           List all notes recursively");
            println!("  read <path>          Read a note");
            println!("  write <path> <text>  Write a note");
            println!(
                "  export <note>        Export a note: --format pdf|docx|png|svg|html|md|xlsx|csv|json|jsonl|xml|latex|ipynb|jira|code (-o file, --brand slug)"
            );
            println!(
                "  export-pdf <note>    Export a note as a shareable PDF (-o file, --brand slug)"
            );
            println!(
                "  repair [--apply]     Find (and with --apply remove) search rows left behind by deleted notes"
            );
            println!("  stats                Show vault statistics");
            println!("  context              Show system context");
        }
    }

    Ok(())
}

/// What `hq vault repair` prints. A dry run lists what it found and changes nothing.
/// Record that the person at this terminal wrote the note. Best effort: with no database yet
/// (or one the daemon has not migrated), the write stands and nothing is recorded.
fn record_owner_write(config: &HqConfig, rel: &str) {
    let Ok(db) = hq_db::Database::open_existing(&config.db_path()) else {
        return;
    };
    hq_tools::provenance::Recorder::new(std::sync::Arc::new(db), config.vault_path.clone())
        .record(rel, &hq_tools::provenance::Writer::owner());
}

fn repair_summary(report: &hq_db::search::RepairReport, apply: bool) -> String {
    let orphans = report.orphan_paths();
    let mode = if apply { "" } else { " (dry run)" };
    let mut out = format!("Search index repair{mode}\n");
    if orphans.is_empty() {
        out.push_str("  Nothing left behind: the index matches the notes on disk.\n");
        return out;
    }
    for (label, found) in [
        (
            "keyword-index rows with no file",
            &report.indexed_without_file,
        ),
        ("embeddings with no note", &report.embeddings_without_note),
        ("cached copies with no file", &report.cached_without_file),
        ("sync records with no file", &report.synced_without_file),
    ] {
        out.push_str(&format!("  {label:<34}{}\n", found.len()));
    }
    let examples: Vec<&str> = orphans.iter().take(5).copied().collect();
    out.push_str(&format!("  e.g. {}\n", examples.join(", ")));
    if apply {
        out.push_str(&format!(
            "Removed what was left behind for {} note(s).\n",
            report.removed
        ));
    } else {
        out.push_str(&format!(
            "{} note(s) have rows left behind. Nothing was changed; run `hq vault repair --apply` to remove them.\n",
            orphans.len()
        ));
    }
    out
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use hq_db::search::RepairReport;

    fn found() -> RepairReport {
        RepairReport {
            indexed_without_file: vec!["Notebooks/a.md".into()],
            embeddings_without_note: vec!["Notebooks/a.md".into(), "Skills/b.md".into()],
            cached_without_file: vec![],
            synced_without_file: vec!["Notebooks/a.md".into()],
            removed: 0,
        }
    }

    #[test]
    fn a_dry_run_lists_counts_and_says_nothing_changed() {
        let text = repair_summary(&found(), false);
        assert!(text.starts_with("Search index repair (dry run)"), "{text}");
        let line = text
            .lines()
            .find(|l| l.contains("embeddings with no note"))
            .unwrap();
        assert!(line.trim_end().ends_with(" 2"), "{line}");
        assert!(text.contains("2 note(s) have rows left behind"), "{text}");
        assert!(text.contains("hq vault repair --apply"), "{text}");
    }

    #[test]
    fn an_applied_run_says_how_many_notes_were_cleaned() {
        let mut report = found();
        report.removed = 2;
        let text = repair_summary(&report, true);
        assert!(text.starts_with("Search index repair\n"), "{text}");
        assert!(
            text.contains("Removed what was left behind for 2 note(s)."),
            "{text}"
        );
    }

    #[test]
    fn nothing_found_says_so() {
        let text = repair_summary(&RepairReport::default(), false);
        assert!(
            text.contains("the index matches the notes on disk"),
            "{text}"
        );
    }
}

#[cfg(test)]
mod lite_tests {
    use super::*;
    use hq_core::config::Profile;

    fn vault_with_notes() -> (tempfile::TempDir, HqConfig) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        std::fs::write(dir.path().join("Notebooks/a.md"), "hello").unwrap();
        std::fs::write(dir.path().join("_system/SOUL.md"), "private").unwrap();
        let cfg = HqConfig {
            vault_path: dir.path().to_path_buf(),
            profile: Profile::Lite,
            ..HqConfig::default()
        };
        (dir, cfg)
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn lite_refuses_hq_folders_through_every_path_in() {
        let (dir, cfg) = vault_with_notes();
        for (sub, a) in [
            ("read", &["_system/SOUL.md"][..]),
            ("read", &["Notebooks/../_system/SOUL.md"]),
            ("read", &[" _system/SOUL.md"]),
            ("write", &["_system/SOUL.md", "x"]),
            ("list", &["_system"]),
            ("tree", &["_system"]),
            ("export", &["_system/SOUL.md", "--format", "md"]),
            ("context", &[]),
        ] {
            assert!(run(&cfg, sub, &args(a)).await.is_err(), "{sub} {a:?}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("_system/SOUL.md")).unwrap(),
            "private"
        );
        assert!(run(&cfg, "read", &args(&["Notebooks/a.md"])).await.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lite_refuses_a_symlink_into_a_hq_folder() {
        let (dir, cfg) = vault_with_notes();
        std::os::unix::fs::symlink(
            dir.path().join("_system"),
            dir.path().join("Notebooks/link"),
        )
        .unwrap();
        assert!(
            run(&cfg, "read", &args(&["Notebooks/link/SOUL.md"]))
                .await
                .is_err()
        );
        assert!(
            run(&cfg, "write", &args(&["Notebooks/link/new.md", "x"]))
                .await
                .is_err()
        );
        assert!(!dir.path().join("_system/new.md").exists());
    }

    #[tokio::test]
    async fn a_terminal_write_is_recorded_as_the_owners() {
        let (dir, mut cfg) = vault_with_notes();
        cfg.profile = Profile::Full;
        // A migrated database must exist for there to be anywhere to record.
        let db = hq_db::Database::open(&cfg.db_path()).unwrap();
        run(
            &cfg,
            "write",
            &args(&["Notebooks/b.md", "typed", "by", "hand"]),
        )
        .await
        .unwrap();

        let raw = std::fs::read_to_string(dir.path().join("Notebooks/b.md")).unwrap();
        let hash = hq_db::search::content_hash(&raw);
        let found = db
            .with_conn(|c| hq_db::provenance::current(c, "Notebooks/b.md", &hash))
            .unwrap()
            .expect("the write was recorded");
        assert_eq!(found.actor_id, "owner");
        assert_eq!(found.source_kind, hq_db::provenance::SourceKind::Human);
    }

    #[tokio::test]
    async fn a_terminal_write_without_a_database_still_writes() {
        let (dir, mut cfg) = vault_with_notes();
        cfg.profile = Profile::Full;
        run(&cfg, "write", &args(&["Notebooks/c.md", "text"]))
            .await
            .unwrap();
        assert!(dir.path().join("Notebooks/c.md").is_file());
    }

    #[tokio::test]
    async fn the_full_profile_is_unchanged() {
        let (_dir, mut cfg) = vault_with_notes();
        cfg.profile = Profile::Full;
        assert!(run(&cfg, "read", &args(&["_system/SOUL.md"])).await.is_ok());
    }
}
