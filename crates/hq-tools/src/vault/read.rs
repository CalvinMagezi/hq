//! Vault read tools: notes, outlines, sections, context, listings, and batch reads.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{note_to_json, validate_path_in_vault};
use crate::registry::HqTool;

/// Read a single note by relative path with line slicing options.
pub struct VaultReadTool {
    vault: Arc<VaultClient>,
}

impl VaultReadTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultReadTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_read"
    }

    fn description(&self) -> &str {
        "Read a vault note by relative path. Supports line range slicing (start_line, end_line) for token efficiency."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path inside the vault (e.g. Notebooks/Projects/foo.md)" },
                "start_line": { "type": "integer", "description": "Optional 1-indexed start line" },
                "end_line": { "type": "integer", "description": "Optional 1-indexed end line" },
                "include_frontmatter": { "type": "boolean", "description": "Include YAML frontmatter metadata (default true)", "default": true }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let start_line = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let end_line = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        validate_path_in_vault(self.vault.vault_path(), path)?;

        let vault = self.vault.clone();
        let path_owned = path.to_string();
        let note = tokio::task::spawn_blocking(move || vault.read_note(&path_owned)).await??;

        let lines: Vec<&str> = note.content.lines().collect();
        let total_lines = lines.len();

        let (sliced_content, eff_start, eff_end) = match (start_line, end_line) {
            (Some(s), Some(e)) if s >= 1 && e >= s => {
                let start_idx = s - 1;
                let end_idx = e.min(total_lines);
                if start_idx < total_lines {
                    (lines[start_idx..end_idx].join("\n"), s, end_idx)
                } else {
                    (String::new(), s, total_lines)
                }
            }
            (Some(s), None) if s >= 1 => {
                let start_idx = s - 1;
                if start_idx < total_lines {
                    (lines[start_idx..].join("\n"), s, total_lines)
                } else {
                    (String::new(), s, total_lines)
                }
            }
            (None, Some(e)) => {
                let end_idx = e.min(total_lines);
                (lines[..end_idx].join("\n"), 1, end_idx)
            }
            _ => (note.content.clone(), 1, total_lines),
        };

        Ok(json!({
            "path": note.path,
            "title": note.title,
            "content": sliced_content,
            "tags": note.tags,
            "start_line": eff_start,
            "end_line": eff_end,
            "total_lines": total_lines,
            "modified_at": note.modified_at.to_rfc3339(),
        }))
    }
}

// ─── VaultOutlineTool ───────────────────────────────────────────

/// Extract structured heading outline (Table of Contents) from a note. Extremely token-efficient.
pub struct VaultOutlineTool {
    vault: Arc<VaultClient>,
}

impl VaultOutlineTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultOutlineTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_outline"
    }

    fn description(&self) -> &str {
        "Extract markdown heading outline (Table of Contents) from a note. Extremely token-efficient for inspecting large notes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        validate_path_in_vault(self.vault.vault_path(), path)?;

        let vault = self.vault.clone();
        let path_owned = path.to_string();
        let (title, outline) =
            tokio::task::spawn_blocking(move || -> Result<(String, Vec<hq_vault::NoteHeading>)> {
                let note = vault.read_note(&path_owned)?;
                let outline = vault.extract_outline(&path_owned)?;
                Ok((note.title, outline))
            })
            .await??;

        Ok(json!({
            "path": path,
            "title": title,
            "headings": outline,
            "count": outline.len()
        }))
    }
}

// ─── VaultReadSectionTool ───────────────────────────────────────

/// Read a specific section by heading title.
pub struct VaultReadSectionTool {
    vault: Arc<VaultClient>,
}

impl VaultReadSectionTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultReadSectionTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_read_section"
    }

    fn description(&self) -> &str {
        "Read only the content under a targeted section heading. Avoids reading the whole file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "heading": { "type": "string", "description": "Heading title (e.g. 'Key Decisions' or '## Key Decisions')" }
            },
            "required": ["path", "heading"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let heading = args
            .get("heading")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        validate_path_in_vault(self.vault.vault_path(), path)?;

        let vault = self.vault.clone();
        let path_owned = path.to_string();
        let heading_owned = heading.to_string();

        let section =
            tokio::task::spawn_blocking(move || -> Result<Option<hq_vault::NoteSection>> {
                vault.read_section(&path_owned, &heading_owned)
            })
            .await??;

        match section {
            Some(sec) => Ok(json!({
                "found": true,
                "path": path,
                "heading": sec.heading,
                "level": sec.level,
                "start_line": sec.start_line,
                "end_line": sec.end_line,
                "content": sec.content,
            })),
            None => Ok(json!({
                "found": false,
                "path": path,
                "heading": heading,
                "message": format!("Heading '{}' not found in note", heading)
            })),
        }
    }
}

// ─── VaultContextTool ───────────────────────────────────────────

/// Read the four system context files: SOUL, MEMORY, PREFERENCES, HEARTBEAT.
pub struct VaultContextTool {
    vault: Arc<VaultClient>,
}

impl VaultContextTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultContextTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_context"
    }

    fn description(&self) -> &str {
        "Read system context files (SOUL.md, MEMORY.md, PREFERENCES.md, HEARTBEAT.md) from the vault _system/ directory."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let vault = self.vault.clone();
        let ctx = tokio::task::spawn_blocking(move || -> Result<Value> {
            let read = |name: &str| -> String {
                vault.read_note(name).map(|n| n.content).unwrap_or_default()
            };
            Ok(json!({
                "soul": read("_system/SOUL.md"),
                "memory": read("_system/MEMORY.md"),
                "preferences": read("_system/PREFERENCES.md"),
                "heartbeat": read("_system/HEARTBEAT.md"),
            }))
        })
        .await??;
        Ok(ctx)
    }
}

// ─── VaultListTool ──────────────────────────────────────────────

/// List markdown files in a vault directory.
pub struct VaultListTool {
    vault: Arc<VaultClient>,
}

impl VaultListTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultListTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_list"
    }

    fn description(&self) -> &str {
        "List markdown files in a vault directory. Optionally recurse into subdirectories."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string", "description": "Relative directory to list (default: root)", "default": "" },
                "recursive": { "type": "boolean", "description": "Recurse into subdirectories", "default": false }
            },
            "required": []
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let dir = args.get("directory").and_then(|v| v.as_str()).unwrap_or("");
        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        validate_path_in_vault(self.vault.vault_path(), dir)?;

        let vault = self.vault.clone();
        let dir_owned = dir.to_string();
        let paths = tokio::task::spawn_blocking(move || {
            if recursive {
                vault.list_notes_recursive(&dir_owned)
            } else {
                vault.list_notes(&dir_owned)
            }
        })
        .await??;

        Ok(json!({ "paths": paths, "count": paths.len() }))
    }
}

// ─── VaultBatchReadTool ─────────────────────────────────────────

/// Read up to 20 notes at once.
pub struct VaultBatchReadTool {
    vault: Arc<VaultClient>,
}

impl VaultBatchReadTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultBatchReadTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_batch_read"
    }

    fn description(&self) -> &str {
        "Read up to 20 vault notes at once. Returns an array of notes (or errors for missing paths)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of relative paths (max 20)",
                    "maxItems": 20
                }
            },
            "required": ["paths"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let paths: Vec<String> = args
            .get("paths")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if paths.len() > 20 {
            bail!("batch_read limited to 20 paths, got {}", paths.len());
        }

        for p in &paths {
            validate_path_in_vault(self.vault.vault_path(), p)?;
        }

        let vault = self.vault.clone();
        let results = tokio::task::spawn_blocking(move || -> Vec<Value> {
            paths
                .iter()
                .map(|p| match vault.read_note(p) {
                    Ok(note) => note_to_json(&note),
                    Err(e) => json!({ "path": p, "error": e.to_string() }),
                })
                .collect()
        })
        .await?;

        Ok(json!({ "notes": results, "count": results.len() }))
    }
}
