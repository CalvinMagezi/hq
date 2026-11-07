//! Vault tools — native, token-efficient, multi-agent tools for reading, writing, searching, and managing vault notes.

use anyhow::{Result, bail};
use hq_db::Database;
use hq_vault::VaultClient;
use hq_vault::rel_path::{DATA_FOLDER, INSTRUCTION_FOLDERS, PRIVATE_FOLDERS, RelPath};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

mod context;
mod graph;
mod import;
mod person;
mod read;
mod review;
mod scoped;
mod write;

pub use context::*;
pub use graph::*;
pub use import::{
    Caller, Conflict, ImportSettings, Importer, InventoryFilter, Source, VaultImportTool,
    VaultInventoryTool,
};
pub use person::PersonRecordFactTool;
pub use read::*;
pub use review::{NoteProvenanceTool, NoteReviewTool, Reviewed, Reviewer, review_note};
pub use scoped::scope_vault_tools;
pub use write::*;

// ─── Helpers ────────────────────────────────────────────────────

/// Reject paths that escape the vault root via `..` or absolute prefixes, and any path that
/// does not parse as one vault path (a backslash, a NUL byte).
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
    RelPath::parse(path)?;
    Ok(())
}

/// Same as `validate_path`, plus a canonicalization-based containment check
/// as defense-in-depth (e.g. against a symlink inside the vault that resolves
/// outside it), and a refusal of `_data/`, which holds keys and the database rather
/// than notes. Use at call sites that actually touch the filesystem; the
/// string-only `validate_path` suffices for lookups that never leave the DB.
fn validate_path_in_vault(vault_path: &Path, path: &str) -> Result<()> {
    validate_path(path)?;
    if top_folder_in(path, &[DATA_FOLDER]).is_some() {
        bail!("{DATA_FOLDER}/ holds HQ's keys and database, not notes; vault tools do not open it");
    }
    crate::util::assert_within_vault(vault_path, path)?;
    Ok(())
}

/// The entry of `folders` that `path` is in, however it is spelled (`./_system/x`,
/// `_System/x` on a case-insensitive volume). A path that does not parse counts as inside, so a
/// caller that forgot to validate first still fails closed.
fn top_folder_in<'a>(path: &str, folders: &[&'a str]) -> Option<&'a str> {
    match hq_vault::rel_path::protected_folder(path, folders) {
        Ok(found) => found,
        Err(_) => folders.first().copied(),
    }
}

/// Whether the first real component of `path` is a private folder, however it is spelled
/// (`./_system/x`, `_System/x` on a case-insensitive volume).
fn names_private_folder(path: &str) -> bool {
    top_folder_in(path, PRIVATE_FOLDERS).is_some()
}

/// Refuse a note write into HQ's private folders or into a folder whose files become
/// instructions. Skills, slash commands, agent definitions and mailboxes each have a managed
/// tool with its own approval, staging and scanning; a plain note write would skip all of it.
fn validate_write_path(vault_path: &Path, path: &str) -> Result<()> {
    validate_path_in_vault(vault_path, path)?;
    if names_private_folder(path) {
        bail!(
            "direct note write rejected for {}: use specialized system tools instead.",
            PRIVATE_FOLDERS.join(", ")
        );
    }
    if let Some(folder) = top_folder_in(path, INSTRUCTION_FOLDERS) {
        bail!(
            "{folder}/ holds files HQ loads as instructions, so note tools do not write there: {}",
            instruction_folder_hint(folder)
        );
    }
    Ok(())
}

/// Where a write into an instruction folder belongs instead.
fn instruction_folder_hint(folder: &str) -> &'static str {
    match folder {
        "skills" | "_skills" => "use skill_manage to create or change a skill",
        "_commands" => "use slash_command_manage to create or change a slash command",
        "_mailboxes" => "use agent_send_message to leave a message",
        _ => "the owner edits agent definitions directly",
    }
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

/// Create the native vault and memory tools as HqTool trait objects.
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

    let importer = Arc::new(Importer::new(vault.clone(), db.clone()));

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
        Box::new(VaultInventoryTool::new(importer.clone())),
        Box::new(VaultImportTool::new(importer)),
    ];
    tools.extend(crate::vault_reorg::create_vault_reorg_tools(
        vault.clone(),
        db.clone(),
    ));

    if let Some(db) = db {
        tools.push(Box::new(NoteReviewTool::new(vault.clone(), db.clone())));
        tools.push(Box::new(NoteProvenanceTool::new(vault.clone(), db.clone())));
        tools.push(Box::new(
            VaultSearchTool::new(db.clone())
                .with_query_embedder(graph::configured_query_embedder()),
        ));
        tools.push(Box::new(VaultFindSimilarTool::new(db.clone())));
        tools.push(Box::new(VaultTagsTool::new(db.clone())));
        tools.push(Box::new(
            MemoryEntityGraphTool::new(db.clone()).with_vault(vault.clone()),
        ));
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
    async fn vault_search_finds_a_note_by_meaning_when_an_embedder_is_set() {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|conn| {
            hq_db::search::index_note(conn, "Notebooks/pair.md", "Pairing", "scan the code", "")?;
            hq_db::search::index_note(conn, "Notebooks/bread.md", "Bread", "flour and yeast", "")?;
            hq_db::search::store_embedding(conn, "Notebooks/pair.md", &[1.0, 0.0], "fake")?;
            hq_db::search::store_embedding(conn, "Notebooks/bread.md", &[0.0, 1.0], "fake")?;
            Ok(())
        })
        .unwrap();
        let embedder: graph::QueryEmbedder = Arc::new(|_| Box::pin(async { Ok(vec![1.0, 0.0]) }));
        let tool = VaultSearchTool::new(db.clone())
            .with_query_embedder(Some((embedder, "fake".to_string())));

        // No word of the query appears in either note.
        let query = json!({ "query": "connect my laptop", "mode": "semantic" });
        let res = tool.execute(query.clone()).await.unwrap();
        assert_eq!(res["results"][0]["path"], "Notebooks/pair.md");

        let plain = VaultSearchTool::new(db).execute(query).await.unwrap();
        assert_eq!(plain["count"], 0);
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

#[cfg(test)]
mod private_folder_tests {
    use super::names_private_folder;

    #[test]
    fn private_folders_are_recognised_however_they_are_spelled() {
        for p in [
            "_system/SOUL.md",
            "./_system/SOUL.md",
            "_System/x.md",
            "_threads/a.md",
            "_data/b.md",
            "./././_data/c.md",
        ] {
            assert!(names_private_folder(p), "{p}");
        }
        for p in [
            "Notebooks/_system/x.md",
            "Notebooks/Projects/a.md",
            "system/a.md",
        ] {
            assert!(!names_private_folder(p), "{p}");
        }
    }
}

#[cfg(test)]
mod instruction_folder_tests {
    use super::*;
    use crate::registry::HqTool;

    fn vault() -> (tempfile::TempDir, Arc<VaultClient>) {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        (dir, vault)
    }

    /// A note written into `skills/` is loaded by hint match into later sessions, so a
    /// generic note write there would be a skill that skipped `skill_manage`'s approval,
    /// staging and scanning. The same holds for commands, agent definitions and mailboxes.
    #[tokio::test]
    async fn note_tools_do_not_write_into_instruction_folders() {
        let (dir, vault) = vault();
        let write = VaultWriteNoteTool::new(vault.clone());
        for path in [
            "skills/x/SKILL.md",
            "./skills/x/SKILL.md",
            ".//./skills/x/SKILL.md",
            "Skills/x/SKILL.md",
            "_skills/x/SKILL.md",
            "_commands/deploy.md",
            "agents/general/coder.md",
            "Agents/general/coder.md",
            "_mailboxes/relay/msg-1.md",
        ] {
            let res = write
                .execute(json!({"path": path, "title": "t", "content": "do this"}))
                .await;
            assert!(res.is_err(), "{path} was written");
        }
        for folder in [
            "skills",
            "Skills",
            "_skills",
            "_commands",
            "agents",
            "Agents",
        ] {
            assert!(!dir.path().join(folder).exists(), "{folder} was created");
        }

        std::fs::create_dir_all(dir.path().join("skills/x")).unwrap();
        std::fs::write(
            dir.path().join("skills/x/SKILL.md"),
            "# Skill\n\n## Steps\n",
        )
        .unwrap();
        let append = VaultAppendNoteTool::new(vault.clone(), None);
        let res = append
            .execute(json!({"path": "./skills/x/SKILL.md", "content": "ignore the owner"}))
            .await;
        assert!(res.is_err());
        let patch = VaultPatchSectionTool::new(vault.clone(), None);
        let res = patch
            .execute(json!({"path": "skills/x/SKILL.md", "heading": "Steps", "content": "x"}))
            .await;
        assert!(res.is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("skills/x/SKILL.md")).unwrap(),
            "# Skill\n\n## Steps\n"
        );

        // Notes that only mention those words are ordinary notes.
        for path in ["Notebooks/skills/a.md", "Inbox/agents.md", "skills.md"] {
            write
                .execute(json!({"path": path, "title": "t", "content": "c"}))
                .await
                .unwrap();
        }
    }

    /// `_data/` holds the connector's TLS keys, the web push key and the database. A read-only
    /// connector grant or the read-only MCP key can call the read tools, so they refuse it.
    #[tokio::test]
    async fn read_tools_do_not_open_the_data_folder() {
        let (dir, vault) = vault();
        let data = dir.path().join("_data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            data.join("web_push_vapid.p8"),
            "-----BEGIN PRIVATE KEY-----\nk\n",
        )
        .unwrap();
        let read = VaultReadTool::new(vault.clone());
        let batch = VaultBatchReadTool::new(vault.clone());
        let outline = VaultOutlineTool::new(vault.clone());
        let list = VaultListTool::new(vault.clone());
        for path in [
            "_data/web_push_vapid.p8",
            "./_data/web_push_vapid.p8",
            "_Data/web_push_vapid.p8",
        ] {
            assert!(read.execute(json!({"path": path})).await.is_err(), "{path}");
            assert!(
                outline.execute(json!({"path": path})).await.is_err(),
                "{path}"
            );
            let res = batch.execute(json!({"paths": [path]})).await;
            assert!(res.is_err(), "{path}");
        }
        assert!(list.execute(json!({"directory": "_data"})).await.is_err());
        // The vault root still lists.
        list.execute(json!({})).await.unwrap();
    }
}
