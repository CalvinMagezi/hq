//! Vault reorganization tools: move, trash, mkdir, frontmatter mutation.
//!
//! The safety rails (protected `_system`/`_threads`/`_data`/`_trash`
//! directories, trash-not-delete, wikilink rewriting) live in
//! `hq_vault::reorg`; these tools add dry-run plumbing and search-index sync.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;
use crate::util::arg_str;

fn arg_dry_run(args: &Value) -> bool {
    args.get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn reindex_moved_note(db: Option<&Arc<Database>>, vault: &VaultClient, from: &str, to: &str) {
    let Some(db) = db else { return };
    let from = from.to_string();
    let to = to.to_string();
    let note = vault.read_note(&to).ok();
    let _ = db.with_conn(move |conn| {
        let _ = hq_db::search::remove_note(conn, &from);
        if let Some(note) = note {
            let tags = note.tags.join(" ");
            let _ = hq_db::search::index_note(conn, &to, &note.title, &note.content, &tags);
        }
        Ok(())
    });
}

fn remove_from_index(db: Option<&Arc<Database>>, path: &str) {
    let Some(db) = db else { return };
    let path = path.to_string();
    let _ = db.with_conn(move |conn| {
        let _ = hq_db::search::remove_note(conn, &path);
        Ok(())
    });
}

pub struct VaultMoveTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

#[async_trait]
impl HqTool for VaultMoveTool {
    fn name(&self) -> &str {
        "vault_move"
    }
    fn description(&self) -> &str {
        "Move or rename a vault note. Rewrites inbound [[wikilinks]] to the new name and updates the search index. Use dry_run to preview which notes reference it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Current relative note path (e.g. Notebooks/Inbox/idea.md)" },
                "to": { "type": "string", "description": "New relative note path (e.g. Notebooks/Projects/idea.md)" },
                "dry_run": { "type": "boolean", "description": "Preview only: report referrers without moving", "default": false }
            },
            "required": ["from", "to"]
        })
    }
    fn category(&self) -> &str {
        "vault"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let from = arg_str(&args, "from");
        let to = arg_str(&args, "to");
        let dry_run = arg_dry_run(&args);
        let vault = self.vault.clone();
        let (f, t) = (from.clone(), to.clone());
        let report =
            tokio::task::spawn_blocking(move || vault.move_note(&f, &t, dry_run)).await??;
        if !dry_run {
            reindex_moved_note(self.db.as_ref(), &self.vault, &from, &to);
        }
        Ok(serde_json::to_value(&report)?)
    }
}

pub struct VaultDeleteTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

#[async_trait]
impl HqTool for VaultDeleteTool {
    fn name(&self) -> &str {
        "vault_delete"
    }
    fn description(&self) -> &str {
        "Delete a vault note by moving it to _trash/ (30-day retention, never a hard delete). Removes it from the search index."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path to trash" },
                "dry_run": { "type": "boolean", "description": "Preview the trash destination without moving", "default": false }
            },
            "required": ["path"]
        })
    }
    fn category(&self) -> &str {
        "vault"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let path = arg_str(&args, "path");
        let dry_run = arg_dry_run(&args);
        let vault = self.vault.clone();
        let p = path.clone();
        let trashed_to =
            tokio::task::spawn_blocking(move || vault.trash_note(&p, dry_run)).await??;
        if !dry_run {
            remove_from_index(self.db.as_ref(), &path);
        }
        Ok(json!({ "trashed_to": trashed_to, "dry_run": dry_run }))
    }
}

pub struct VaultFrontmatterUpdateTool {
    vault: Arc<VaultClient>,
}

#[async_trait]
impl HqTool for VaultFrontmatterUpdateTool {
    fn name(&self) -> &str {
        "vault_frontmatter_update"
    }
    fn description(&self) -> &str {
        "Set or remove frontmatter keys on a vault note without touching its body."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "set": { "type": "object", "description": "Keys to set (string, number, bool, or list values)", "default": {} },
                "remove": { "type": "array", "items": { "type": "string" }, "description": "Keys to remove", "default": [] }
            },
            "required": ["path"]
        })
    }
    fn category(&self) -> &str {
        "vault"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let path = arg_str(&args, "path");
        let set: std::collections::HashMap<String, serde_yaml::Value> = args
            .get("set")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let remove: Vec<String> = args
            .get("remove")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let vault = self.vault.clone();
        let p = path.clone();
        tokio::task::spawn_blocking(move || vault.update_frontmatter(&p, set, remove)).await??;
        Ok(json!({ "updated": path }))
    }
}

/// The four reorg tools, sharing one client and optional index DB.
pub fn create_vault_reorg_tools(
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(VaultMoveTool {
            vault: vault.clone(),
            db: db.clone(),
        }),
        Box::new(VaultDeleteTool {
            vault: vault.clone(),
            db,
        }),
        Box::new(VaultFrontmatterUpdateTool { vault }),
    ]
}
