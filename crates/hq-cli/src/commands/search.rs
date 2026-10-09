use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;

/// Search vault notes by keyword or content.
pub async fn run(config: &HqConfig, query: &str, limit: usize, json: bool) -> Result<()> {
    if query.is_empty() {
        anyhow::bail!("Usage: hq search <query> [--limit N]");
    }

    if !json {
        println!("Searching vault for: \"{}\"\n", query);
    }

    let db_path = config.db_path();
    if !db_path.exists() {
        if json || config.profile.is_lite() {
            anyhow::bail!("the search index is not built yet; start HQ once or run `hq reindex`");
        }
        println!("Search index not built. Run `hq setup` first.");
        println!("Falling back to filesystem search...\n");
        return filesystem_search(config, query, limit);
    }

    let db = Database::open(&db_path)?;
    let lite = config.profile.is_lite();
    let mut results: Vec<hq_core::types::SearchResult> =
        db.with_conn(|conn| hq_db::search::keyword_search(conn, query, if lite { limit.saturating_mul(5) } else { limit }))?;
    if config.profile.is_lite() {
        // By name and by where the note really is: a link in a normal folder can lead into a
        // hidden one, and the index follows links.
        results.retain(|r| {
            !hq_web::lite_hides(&r.note_path)
                && !hq_web::lite_hides_resolved(&config.vault_path, &config.vault_path.join(r.note_path.trim()))
        });
        results.truncate(limit);
    }
    if json {
        let hits: Vec<_> = results
            .iter()
            .map(|r| serde_json::json!({ "path": r.note_path, "score": r.relevance, "snippet": r.snippet }))
            .collect();
        println!("{}", serde_json::json!({ "query": query, "results": hits }));
        return Ok(());
    }

    if results.is_empty() {
        println!("No results found.");
        println!("Try a broader search or check if notes are indexed: hq status");
    } else {
        println!("Found {} result(s):\n", results.len());
        for result in &results {
            println!("  {} (score: {:.2})", result.note_path, result.relevance);
            if !result.snippet.is_empty() {
                let snippet = result.snippet.chars().take(120).collect::<String>();
                println!("    {}", snippet);
            }
            println!();
        }
    }

    Ok(())
}

/// Force a full rebuild of the FTS index, bypassing the daemon's 30-minute
/// incremental tick — useful right after a bulk vault write (migration,
/// rsync) that would otherwise sit unsearchable for up to 30 minutes.
pub async fn reindex(config: &HqConfig) -> Result<()> {
    let db_path = config.db_path();
    if !db_path.exists() {
        anyhow::bail!("Search index not built. Run `hq setup` first.");
    }

    let notebooks_dir = config.vault_path.join("Notebooks");
    let db = Database::open(&db_path)?;
    let (indexed, errors) =
        db.with_conn(|conn| hq_db::search::rebuild_index(conn, &notebooks_dir))?;

    println!("Reindexed {indexed} note(s), {errors} error(s).");
    Ok(())
}

/// Fallback: scan vault files for the query string.
fn filesystem_search(config: &HqConfig, query: &str, limit: usize) -> Result<()> {
    let vault_path = &config.vault_path;
    let query_lower = query.to_lowercase();
    let mut found = 0;

    fn walk_and_search(
        dir: &std::path::Path,
        vault_root: &std::path::Path,
        query: &str,
        found: &mut usize,
        limit: usize,
    ) -> Result<()> {
        if *found >= limit {
            return Ok(());
        }
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            if *found >= limit {
                break;
            }
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !name.starts_with('.') && name != "_data" && name != "_embeddings" {
                    walk_and_search(&path, vault_root, query, found, limit)?;
                }
            } else if path.extension().is_some_and(|ext| ext == "md")
                && let Ok(content) = std::fs::read_to_string(&path)
                && content.to_lowercase().contains(query)
            {
                let rel = path
                    .strip_prefix(vault_root)
                    .unwrap_or(&path)
                    .to_string_lossy();

                // Find the matching line
                let snippet = content
                    .lines()
                    .find(|line| line.to_lowercase().contains(query))
                    .unwrap_or("")
                    .trim();

                println!("  {}", rel);
                if !snippet.is_empty() {
                    let truncated: String = snippet.chars().take(120).collect();
                    println!("    {}", truncated);
                }
                println!();
                *found += 1;
            }
        }

        Ok(())
    }

    walk_and_search(vault_path, vault_path, &query_lower, &mut found, limit)?;

    if found == 0 {
        println!("No results found.");
    } else {
        println!("Found {} result(s) (filesystem search)", found);
    }

    Ok(())
}
