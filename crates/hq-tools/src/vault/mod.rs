//! Vault tools — native, token-efficient, multi-agent tools for reading, writing, searching, and managing vault notes.

use anyhow::{Result, bail};
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

mod context;
mod graph;
mod person;
mod read;
mod scoped;
mod write;

pub use context::*;
pub use graph::*;
pub use person::PersonRecordFactTool;
pub use read::*;
pub use scoped::scope_vault_tools;
pub use write::*;

// ─── Helpers ────────────────────────────────────────────────────

/// Reject paths that escape the vault root via `..` or absolute prefixes.
fn validate_path(path: &str) -> Result<()> {
    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        bail!("path traversal not allowed: {}", path);
    }
    if path.ends_with('/') || path.ends_with('\\') {
        bail!(
            "path must point to a file, not a directory: '{}'. Use vault_search or vault_list to browse folders.",
            path
        );
    }
    Ok(())
}

/// Same as `validate_path`, plus a canonicalization-based containment check
/// as defense-in-depth (e.g. against a symlink inside the vault that resolves
/// outside it). Use at call sites that actually touch the filesystem; the
/// string-only `validate_path` suffices for lookups that never leave the DB.
fn validate_path_in_vault(vault_path: &Path, path: &str) -> Result<()> {
    validate_path(path)?;
    crate::util::assert_within_vault(vault_path, path)?;
    Ok(())
}

fn validate_write_path(vault_path: &Path, path: &str) -> Result<()> {
    validate_path_in_vault(vault_path, path)?;
    if path.starts_with("_system/") || path == "_system" {
        bail!(
            "direct note write rejected for _system/ directory: use specialized system tools instead."
        );
    }
    Ok(())
}

fn note_to_json(note: &hq_core::types::Note) -> Value {
    json!({
        "path": note.path,
        "title": note.title,
        "content": note.content,
        "tags": note.tags,
        "modified_at": note.modified_at.to_rfc3339(),
    })
}

fn sync_note_index_and_cache(
    db: Option<&Arc<Database>>,
    path: &str,
    content: &str,
    title: &str,
    tags: &[String],
) {
    let Some(db) = db else { return };
    let path_owned = path.to_string();
    let content_owned = content.to_string();
    let title_owned = title.to_string();
    let tags_str = tags.join(" ");
    let token_count = ((content_owned.len() * 2).div_ceil(7)) as i64;

    let _ = db.with_conn(move |conn| {
        let _ =
            hq_db::search::index_note(conn, &path_owned, &title_owned, &content_owned, &tags_str);
        let _ = hq_db::vault_cache::write_note_cached(
            conn,
            &path_owned,
            &content_owned,
            &title_owned,
            None,
            Some(token_count),
        );
        Ok(())
    });
}

// ─── Tool Factory ────────────────────────────────────────────────

/// Create all 15 native vault and memory tools as HqTool trait objects.
/// Non-fatal: returns an empty vec if VaultClient cannot be opened.
pub fn create_vault_tools(
    vault_path: std::path::PathBuf,
    db: Option<std::sync::Arc<hq_db::Database>>,
) -> Vec<Box<dyn crate::registry::HqTool>> {
    let vault = match VaultClient::new(vault_path) {
        Ok(v) => std::sync::Arc::new(v),
        Err(e) => {
            tracing::warn!("create_vault_tools: could not open VaultClient: {e}");
            return vec![];
        }
    };

    let write_tool: Box<dyn crate::registry::HqTool> = if let Some(ref db) = db {
        Box::new(VaultWriteNoteTool::new_with_cache(
            vault.clone(),
            db.clone(),
        ))
    } else {
        Box::new(VaultWriteNoteTool::new(vault.clone()))
    };

    let mut tools: Vec<Box<dyn crate::registry::HqTool>> = vec![
        Box::new(VaultReadTool::new(vault.clone())),
        Box::new(VaultOutlineTool::new(vault.clone())),
        Box::new(VaultReadSectionTool::new(vault.clone())),
        Box::new(VaultContextTool::new(vault.clone())),
        Box::new(VaultListTool::new(vault.clone())),
        Box::new(VaultBatchReadTool::new(vault.clone())),
        Box::new(VaultAppendNoteTool::new(vault.clone(), db.clone())),
        Box::new(VaultPatchSectionTool::new(vault.clone(), db.clone())),
        Box::new(VaultLinksTool::new(vault.clone())),
        Box::new(VaultBacklinksTool::new(vault.clone(), db.clone())),
        Box::new(
            PersonRecordFactTool::new(vault.clone(), db.clone())
                .with_registered_people(person::configured_people()),
        ),
        write_tool,
        Box::new(ContextPacketTool::new(vault.clone(), db.clone())),
    ];
    tools.extend(crate::vault_reorg::create_vault_reorg_tools(
        vault.clone(),
        db.clone(),
    ));

    if let Some(db) = db {
        tools.push(Box::new(VaultSearchTool::new(db.clone())));
        tools.push(Box::new(VaultFindSimilarTool::new(db.clone())));
        tools.push(Box::new(VaultTagsTool::new(db.clone())));
        tools.push(Box::new(MemoryEntityGraphTool::new(db.clone()).with_vault(vault.clone())));
        tools.push(Box::new(VaultReindexTool::new(
            db,
            vault.vault_path().to_path_buf(),
        )));
    }

    tools
}

#[cfg(test)]
mod vault_tools_tests {
    use super::*;
    use crate::registry::HqTool;
    use tempfile::TempDir;

    #[test]
    fn validate_path_in_vault_accepts_normal_relative_path() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("existing.md"), "hi").unwrap();
        assert!(validate_path_in_vault(dir.path(), "existing.md").is_ok());
    }

    #[test]
    fn validate_path_in_vault_accepts_not_yet_existing_path() {
        let dir = TempDir::new().unwrap();
        // Note-create tools call this before the file exists on disk.
        assert!(validate_path_in_vault(dir.path(), "Notebooks/new-note.md").is_ok());
    }

    #[test]
    fn validate_path_in_vault_rejects_dotdot_traversal() {
        let dir = TempDir::new().unwrap();
        assert!(validate_path_in_vault(dir.path(), "../outside.md").is_err());
    }

    #[test]
    fn validate_path_in_vault_rejects_absolute_path() {
        let dir = TempDir::new().unwrap();
        assert!(validate_path_in_vault(dir.path(), "/etc/passwd").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn validate_path_in_vault_rejects_symlink_escape() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.md"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("link.md"))
            .unwrap();
        assert!(validate_path_in_vault(dir.path(), "link.md").is_err());
    }

    #[tokio::test]
    async fn test_vault_outline_and_section_tools() {
        let dir = TempDir::new().unwrap();
        let vault_path = dir.path().to_path_buf();
        let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

        let note = hq_core::types::Note {
            title: "Test Project".to_string(),
            content: "# Test Project\n\n## Overview\nProject overview text.\n\n## Tasks\n- Task A\n- Task B".to_string(),
            path: "Notebooks/Projects/test.md".to_string(),
            frontmatter: std::collections::HashMap::new(),
            note_type: None,
            tags: vec!["project".to_string()],
            pinned: false,
            source: None,
            embedding_status: None,
            created_at: None,
            updated_at: None,
            modified_at: chrono::Utc::now(),
        };
        vault
            .write_note("Notebooks/Projects/test.md", &note)
            .unwrap();

        // 1. Outline tool
        let outline_tool = VaultOutlineTool::new(vault.clone());
        let res = outline_tool
            .execute(json!({ "path": "Notebooks/Projects/test.md" }))
            .await
            .unwrap();
        assert_eq!(res["count"], 3);

        // 2. Read Section tool
        let section_tool = VaultReadSectionTool::new(vault.clone());
        let sec_res = section_tool
            .execute(json!({
                "path": "Notebooks/Projects/test.md",
                "heading": "Tasks"
            }))
            .await
            .unwrap();
        assert_eq!(sec_res["found"], true);
        assert!(sec_res["content"].as_str().unwrap().contains("Task A"));

        // 3. Append Note tool
        let append_tool = VaultAppendNoteTool::new(vault.clone(), None);
        let app_res = append_tool
            .execute(json!({
                "path": "Notebooks/Projects/test.md",
                "heading": "Tasks",
                "content": "- Task C"
            }))
            .await
            .unwrap();
        assert_eq!(app_res["ok"], true);

        let read_tool = VaultReadTool::new(vault.clone());
        let read_res = read_tool
            .execute(json!({ "path": "Notebooks/Projects/test.md" }))
            .await
            .unwrap();
        assert!(read_res["content"].as_str().unwrap().contains("- Task C"));

        // 4. Patch Section tool
        let patch_tool = VaultPatchSectionTool::new(vault.clone(), None);
        let patch_res = patch_tool
            .execute(json!({
                "path": "Notebooks/Projects/test.md",
                "heading": "Overview",
                "content": "Updated overview details."
            }))
            .await
            .unwrap();
        assert_eq!(patch_res["ok"], true);

        let updated_note = vault.read_note("Notebooks/Projects/test.md").unwrap();
        assert!(updated_note.content.contains("Updated overview details."));
    }

    fn seed_vault_cache_row(db: &Database, path: &str, title: &str, mtime: i64) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO vault_cache (path, mtime, hash, title, content_preview) VALUES (?1, ?2, 'h', ?3, 'preview')",
                rusqlite::params![path, mtime, title],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn vault_search_recent_mode_orders_newest_first() {
        let db = Arc::new(Database::open_memory().unwrap());
        seed_vault_cache_row(&db, "Notebooks/old.md", "Old", 100);
        seed_vault_cache_row(&db, "Notebooks/new.md", "New", 999_999_999_999);

        let tool = VaultSearchTool::new(db);
        let res = tool
            .execute(json!({ "mode": "recent", "limit": 5 }))
            .await
            .unwrap();
        let results = res["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["path"], "Notebooks/new.md");
        assert!(results[0]["last_touched"].is_string());
        assert_eq!(results[1]["path"], "Notebooks/old.md");
    }

    #[tokio::test]
    async fn vault_search_sort_by_recency_reorders_keyword_matches() {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|conn| {
            hq_db::search::index_note(conn, "Notebooks/a.md", "Alpha", "shared keyword here", "")?;
            hq_db::search::index_note(conn, "Notebooks/b.md", "Beta", "shared keyword here", "")?;
            Ok(())
        })
        .unwrap();
        seed_vault_cache_row(&db, "Notebooks/a.md", "Alpha", 100);
        seed_vault_cache_row(&db, "Notebooks/b.md", "Beta", 999_999_999_999);

        let tool = VaultSearchTool::new(db);
        let res = tool
            .execute(json!({ "query": "keyword", "mode": "keyword", "sort_by": "recency" }))
            .await
            .unwrap();
        let results = res["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["path"], "Notebooks/b.md");
    }

    #[tokio::test]
    async fn vault_search_recent_mode_needs_no_query() {
        let db = Arc::new(Database::open_memory().unwrap());
        seed_vault_cache_row(&db, "Notebooks/only.md", "Only", 100);

        let tool = VaultSearchTool::new(db);
        let res = tool.execute(json!({ "mode": "recent" })).await.unwrap();
        assert_eq!(res["count"], 1);
    }
}
