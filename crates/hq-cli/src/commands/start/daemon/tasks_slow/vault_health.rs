//! Vault integrity checking: notes, frontmatter, wikilinks, orphans, duplicates.

use anyhow::Result;
use hq_db::Database;
use hq_vault::remediation::{count_notes_and_stems, extract_wikilinks};
use std::path::Path;
use tracing::info;

use super::super::helpers::ensure_dir;

/// Full vault integrity check, written to `_system/VAULT-HEALTH.md`. Wikilinks
/// are counted only; graph_links rows for them were never read back.
pub async fn run_vault_health(vault_path: &Path, db: &Database) -> Result<()> {
    let sys_dir = vault_path.join("_system");
    ensure_dir(&sys_dir);

    let notebooks_dir = vault_path.join("Notebooks");
    let mut total_notes = 0u32;
    let mut broken_frontmatter = 0u32;
    let mut total_links = 0u32;
    let mut dead_links = 0u32;
    let mut all_note_stems: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut note_paths: Vec<std::path::PathBuf> = Vec::new();

    // Pass 1: Count notes, check frontmatter, collect stems
    if notebooks_dir.exists() {
        count_notes_and_stems(
            &notebooks_dir,
            &mut total_notes,
            &mut broken_frontmatter,
            &mut all_note_stems,
            &mut note_paths,
        );
    }

    // Pass 2: Scan wikilinks
    let mut inbound_links = std::collections::HashSet::new();
    let mut outbound_notes = std::collections::HashSet::new();
    for path in &note_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            let links = extract_wikilinks(&content);
            if !links.is_empty() {
                outbound_notes.insert(path.to_string_lossy().to_string());
            }
            for link_target in &links {
                total_links += 1;
                let target_exists = all_note_stems.contains(&link_target.to_lowercase());
                if !target_exists {
                    dead_links += 1;
                }
                inbound_links.insert(link_target.to_lowercase());
            }
        }
    }

    // Pass 3: Orphan detection
    let mut orphans = Vec::new();
    for path in &note_paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let path_str = path.to_string_lossy().to_string();
        if !inbound_links.contains(&stem) && !outbound_notes.contains(&path_str) {
            orphans.push(path_str);
        }
    }
    let orphan_count = orphans.len();

    // Pass 4: Duplicate title detection
    let mut stems_to_paths: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for path in &note_paths {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            stems_to_paths
                .entry(stem.to_lowercase())
                .or_default()
                .push(path.to_string_lossy().to_string());
        }
    }
    let duplicates: Vec<(String, Vec<String>)> = stems_to_paths
        .into_iter()
        .filter(|(_, p)| p.len() > 1)
        .collect();
    let duplicate_count = duplicates.len();

    let db_stats = db.with_conn(hq_db::search::get_stats);
    let indexed = db_stats.as_ref().map(|s| s.fts_count).unwrap_or(0);
    let embedded = db_stats.as_ref().map(|s| s.embedding_count).unwrap_or(0);

    let now = chrono::Utc::now().to_rfc3339();
    let link_health = if total_links > 0 {
        ((total_links - dead_links) as f64 / total_links as f64) * 100.0
    } else {
        100.0
    };
    let mut content = format!(
        "---\nchecked_at: {now}\ntotal_notes: {total_notes}\n\
         broken_frontmatter: {broken_frontmatter}\nindexed: {indexed}\n\
         embedded: {embedded}\ntotal_links: {total_links}\n\
         dead_links: {dead_links}\norphans: {orphan_count}\n\
         duplicates: {duplicate_count}\n---\n\n# Vault Health\n\nLast check: {now}\n\n\
         - **Total notes**: {total_notes}\n\
         - **Broken frontmatter**: {broken_frontmatter}\n\
         - **Indexed (FTS5)**: {indexed}\n\
         - **Embedded**: {embedded}\n\
         - **Wikilinks**: {total_links} total, {dead_links} dead\n\
         - **Link health**: {link_health:.1}%\n\
         - **Orphaned notes**: {orphan_count}\n\
         - **Potential duplicates**: {duplicate_count}\n"
    );

    // List orphaned notes (top 20)
    if !orphans.is_empty() {
        content.push_str("\n## Orphaned Notes\n\nNotes with no inbound or outbound wikilinks.\n\n");
        for path in orphans.iter().take(20) {
            let display = path
                .strip_prefix(vault_path.to_string_lossy().as_ref())
                .unwrap_or(path)
                .trim_start_matches('/');
            content.push_str(&format!("- `{display}`\n"));
        }
        if orphan_count > 20 {
            content.push_str(&format!("\n*...and {} more*\n", orphan_count - 20));
        }
    }

    // List duplicate titles
    if !duplicates.is_empty() {
        content.push_str(
            "\n## Potential Duplicates\n\nNotes with the same filename in different folders.\n\n",
        );
        for (stem, paths) in duplicates.iter().take(20) {
            content.push_str(&format!("**{stem}**:\n"));
            for p in paths {
                let display = p
                    .strip_prefix(vault_path.to_string_lossy().as_ref())
                    .unwrap_or(p)
                    .trim_start_matches('/');
                content.push_str(&format!("  - `{display}`\n"));
            }
        }
    }

    std::fs::write(sys_dir.join("VAULT-HEALTH.md"), content)?;
    info!(
        total_notes,
        broken_frontmatter,
        indexed,
        embedded,
        total_links,
        dead_links,
        orphan_count,
        duplicate_count,
        "vault-health: check complete"
    );
    Ok(())
}
