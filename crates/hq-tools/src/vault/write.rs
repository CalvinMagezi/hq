//! Vault write tools: writing notes, appending, and patching sections.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{sync_note_index_and_cache, validate_write_path};
use crate::registry::HqTool;

/// Write or update a vault note.
pub struct VaultWriteNoteTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultWriteNoteTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault, db: None }
    }

    pub fn new_with_cache(vault: Arc<VaultClient>, db: Arc<Database>) -> Self {
        Self {
            vault,
            db: Some(db),
        }
    }
}

#[async_trait]
impl HqTool for VaultWriteNoteTool {
    fn name(&self) -> &str {
        "vault_write_note"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Put durable knowledge in the vault, not conversation scratch. Use frontmatter and link related notes so the graph stays navigable. _system, _threads, _data, and _trash are protected from vault tools like this one — a raw bash write can still reach them.",
        )
    }

    fn description(&self) -> &str {
        "Write or overwrite a markdown note in the vault. Atomically updates FTS search index and token \
         cache. For knowledge/reference material, not actionable work: an item that needs a status and an \
         owner belongs in the native task system instead (task_create), or promote an existing note with \
         task_create_from_note."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative markdown note path (e.g. Notebooks/Projects/foo.md)" },
                "title": { "type": "string", "description": "Note title" },
                "content": { "type": "string", "description": "Markdown content body" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags", "default": [] }
            },
            "required": ["path", "title", "content"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        validate_write_path(self.vault.vault_path(), &path)?;

        let hq_vault::frontmatter::MergedNote {
            frontmatter,
            title,
            content,
            tags,
        } = hq_vault::frontmatter::merge_leading_frontmatter(title, &content, tags);

        let note = hq_core::types::Note {
            title: title.clone(),
            content: content.clone(),
            path: path.clone(),
            frontmatter,
            note_type: None,
            tags: tags.clone(),
            pinned: false,
            source: None,
            embedding_status: None,
            created_at: None,
            updated_at: None,
            modified_at: chrono::Utc::now(),
        };

        let vault = self.vault.clone();
        let path_ret = path.clone();
        tokio::task::spawn_blocking(move || vault.write_note(&path, &note)).await??;

        sync_note_index_and_cache(self.db.as_ref(), &path_ret, &content, &title, &tags);
        crate::provenance::record_tool_write(
            self.db.as_ref(),
            self.vault.vault_path(),
            &path_ret,
            &args,
        )
        .await;

        let abs_path = self.vault.vault_path().join(&path_ret);
        Ok(json!({ "ok": true, "path": path_ret, "abs_path": abs_path.to_string_lossy() }))
    }
}

// ─── VaultAppendNoteTool ────────────────────────────────────────

/// Append content to a note, optionally under a specific heading.
pub struct VaultAppendNoteTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultAppendNoteTool {
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for VaultAppendNoteTool {
    fn name(&self) -> &str {
        "vault_append_note"
    }

    fn description(&self) -> &str {
        "Append text to a vault note, optionally targeted under a specific heading. Avoids full file rewrites."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "content": { "type": "string", "description": "Text content to append" },
                "heading": { "type": "string", "description": "Optional section heading title under which to append" }
            },
            "required": ["path", "content"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let heading = args.get("heading").and_then(|v| v.as_str());

        validate_write_path(self.vault.vault_path(), &path)?;

        let vault = self.vault.clone();
        let path_owned = path.clone();
        let content_owned = content.clone();
        let heading_owned = heading.map(|s| s.to_string());

        let (title, updated_content, tags) =
            tokio::task::spawn_blocking(move || -> Result<(String, String, Vec<String>)> {
                vault.append_to_note(&path_owned, &content_owned, heading_owned.as_deref())?;
                let note = vault.read_note(&path_owned)?;
                Ok((note.title, note.content, note.tags))
            })
            .await??;

        sync_note_index_and_cache(self.db.as_ref(), &path, &updated_content, &title, &tags);
        crate::provenance::record_tool_write(
            self.db.as_ref(),
            self.vault.vault_path(),
            &path,
            &args,
        )
        .await;

        Ok(json!({ "ok": true, "path": path, "target_heading": heading }))
    }
}

// ─── VaultPatchSectionTool ──────────────────────────────────────

/// Replace content under a heading without modifying surrounding sections or frontmatter.
pub struct VaultPatchSectionTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultPatchSectionTool {
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for VaultPatchSectionTool {
    fn name(&self) -> &str {
        "vault_patch_section"
    }

    fn description(&self) -> &str {
        "Replace body text under a targeted section heading, preserving frontmatter and all other sections."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "heading": { "type": "string", "description": "Section heading title to patch" },
                "content": { "type": "string", "description": "New body content for this section" }
            },
            "required": ["path", "heading", "content"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let heading = args
            .get("heading")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        validate_write_path(self.vault.vault_path(), &path)?;

        let vault = self.vault.clone();
        let path_owned = path.clone();
        let heading_owned = heading.clone();
        let content_owned = content.clone();

        let (title, updated_content, tags) =
            tokio::task::spawn_blocking(move || -> Result<(String, String, Vec<String>)> {
                vault.patch_section(&path_owned, &heading_owned, &content_owned)?;
                let note = vault.read_note(&path_owned)?;
                Ok((note.title, note.content, note.tags))
            })
            .await??;

        sync_note_index_and_cache(self.db.as_ref(), &path, &updated_content, &title, &tags);
        crate::provenance::record_tool_write(
            self.db.as_ref(),
            self.vault.vault_path(),
            &path,
            &args,
        )
        .await;

        Ok(json!({ "ok": true, "path": path, "patched_heading": heading }))
    }
}

#[cfg(test)]
mod note_parts_tests {
    use super::*;

    #[tokio::test]
    async fn frontmatter_inside_content_becomes_the_notes_own() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        let tool = VaultWriteNoteTool::new(vault.clone());
        let content = "---\ntitle: \"Edge notes\"\nstatus: draft\ntags:\n  - edge-ai\n  - android\n---\n\n# Edge notes\n\nBody.\n";
        tool.execute(json!({
            "path": "Notebooks/x.md",
            "title": "",
            "content": content,
            "tags": ["android", "research"],
        }))
        .await
        .unwrap();

        let raw = std::fs::read_to_string(dir.path().join("Notebooks/x.md")).unwrap();
        assert_eq!(
            raw.matches("---").count(),
            2,
            "one frontmatter block:\n{raw}"
        );
        let note = vault.read_note("Notebooks/x.md").unwrap();
        assert_eq!(note.title, "Edge notes");
        assert_eq!(note.tags, ["edge-ai", "android", "research"]);
        assert_eq!(
            note.frontmatter.get("status").and_then(|v| v.as_str()),
            Some("draft")
        );
        assert!(note.content.starts_with("# Edge notes"), "{}", note.content);
    }

    #[test]
    fn tags_given_as_an_argument_are_written_to_the_file() {
        let parts = hq_vault::frontmatter::merge_leading_frontmatter(
            "T".into(),
            "plain body",
            vec!["a".into()],
        );
        assert_eq!(parts.tags, ["a"]);
        assert!(parts.frontmatter.contains_key("tags"));
        assert_eq!(parts.content, "plain body");
    }

    #[test]
    fn content_whose_frontmatter_does_not_parse_is_kept_as_written() {
        let content = "---\n: : not yaml [\n---\nbody";
        let parts =
            hq_vault::frontmatter::merge_leading_frontmatter("T".into(), content, Vec::new());
        assert_eq!(parts.content, content);
    }
}
