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
        // Its links belong to the note, not the old path.
        let _ = hq_db::search::move_graph_links(conn, &from, &to);
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
        let _ = hq_db::search::remove_path_everywhere(conn, &path);
        Ok(())
    });
}

pub struct VaultMoveTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultMoveTool {
    fn recorder(&self) -> Option<crate::provenance::Recorder> {
        self.db.as_ref().map(|db| {
            crate::provenance::Recorder::new(db.clone(), self.vault.vault_path().to_path_buf())
        })
    }
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
        let recorder = self.recorder();
        // Moving rewrites links in other notes. Fingerprint them first so only the notes the move
        // really changed are credited to the caller.
        let before = match (&recorder, dry_run) {
            (Some(recorder), false) => {
                let (vault, f, t) = (self.vault.clone(), from.clone(), to.clone());
                let recorder = recorder.clone();
                tokio::task::spawn_blocking(move || {
                    let mut rels = vault
                        .move_note(&f, &t, true)
                        .map(|r| r.referrers)
                        .unwrap_or_default();
                    rels.push(f);
                    recorder.fingerprint(&rels)
                })
                .await
                .ok()
            }
            _ => None,
        };
        let report =
            tokio::task::spawn_blocking(move || vault.move_note(&f, &t, dry_run)).await??;
        if !dry_run {
            reindex_moved_note(self.db.as_ref(), &self.vault, &from, &to);
            if let (Some(recorder), Some(before)) = (recorder, before) {
                let (referrers, args, from, to) = (
                    report.referrers.clone(),
                    args.clone(),
                    from.clone(),
                    to.clone(),
                );
                let _ = tokio::task::spawn_blocking(move || {
                    recorder.after_move(&from, &to, &referrers, &before, &args)
                })
                .await;
            }
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
            if let Some(db) = self.db.as_ref() {
                crate::provenance::Recorder::new(db.clone(), self.vault.vault_path().to_path_buf())
                    .after_delete(&path);
            }
        }
        Ok(json!({ "trashed_to": trashed_to, "dry_run": dry_run }))
    }
}

pub struct VaultFrontmatterUpdateTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
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
        crate::provenance::record_tool_write(
            self.db.as_ref(),
            self.vault.vault_path(),
            &path,
            &args,
        )
        .await;
        Ok(json!({ "updated": path }))
    }
}

pub struct VaultPinTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

#[async_trait]
impl HqTool for VaultPinTool {
    fn name(&self) -> &str {
        "vault_pin"
    }
    fn description(&self) -> &str {
        "Pin a vault note (or unpin one you pinned) so it shows in the owner's pinned notes. A note an agent pins appears there as a suggestion, marked with who pinned it; it joins HQ's own context only once the owner pins it themselves. An agent cannot unpin the owner's pins."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "pin": { "type": "boolean", "description": "true to pin (default), false to unpin", "default": true }
            },
            "required": ["path"]
        })
    }
    fn category(&self) -> &str {
        "vault"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let path = arg_str(&args, "path");
        let pin = args.get("pin").and_then(Value::as_bool).unwrap_or(true);
        if !pin {
            self.refuse_owner_pin(&path)?;
        }
        let (set, remove) = if pin {
            let set = std::collections::HashMap::from([(
                "pinned".to_string(),
                serde_yaml::Value::Bool(true),
            )]);
            (set, Vec::new())
        } else {
            (std::collections::HashMap::new(), vec!["pinned".to_string()])
        };
        let vault = self.vault.clone();
        let p = path.clone();
        tokio::task::spawn_blocking(move || vault.update_frontmatter(&p, set, remove)).await??;
        self.record_as_agent(&path, &args);
        Ok(json!({
            "path": path,
            "pinned": pin,
            "note": if pin {
                "pinned; it shows in the owner's pinned notes as your suggestion until they pin it themselves"
            } else {
                "unpinned"
            },
        }))
    }
}

/// Who a pin made through a tool is recorded as when the caller did not say: never the owner.
const UNATTRIBUTED_PINNER: &str = "an unattributed agent";

impl VaultPinTool {
    /// Every call of this tool is an agent's, so a caller HQ could not attribute is
    /// recorded as one rather than left unknown, which would make its pin trusted.
    fn record_as_agent(&self, path: &str, args: &Value) {
        let Some(db) = &self.db else { return };
        let mut writer = crate::provenance::resolve(args, db);
        if writer.source_kind == hq_db::provenance::SourceKind::Unknown {
            writer.source_kind = hq_db::provenance::SourceKind::Agent;
            writer
                .actor_id
                .get_or_insert_with(|| UNATTRIBUTED_PINNER.to_string());
        }
        crate::provenance::Recorder::new(db.clone(), self.vault.vault_path().to_path_buf())
            .record(path, &writer);
    }

    /// A pin the owner made (any pin not suggested by an agent) is theirs to remove.
    fn refuse_owner_pin(&self, path: &str) -> Result<()> {
        let Some(db) = &self.db else { return Ok(()) };
        let Ok(raw) = std::fs::read_to_string(self.vault.vault_path().join(path)) else {
            return Ok(());
        };
        let pinned = hq_core::frontmatter_utils::split_frontmatter(&raw)
            .0
            .and_then(|fm| serde_yaml::from_str::<serde_yaml::Mapping>(fm).ok())
            .and_then(|fm| fm.get("pinned").and_then(serde_yaml::Value::as_bool))
            .unwrap_or(false);
        if !pinned {
            return Ok(());
        }
        let suggested = db.with_conn(|c| hq_db::provenance::pin_suggested_by(c, path, &raw))?;
        if suggested.is_none() {
            anyhow::bail!("{path} is pinned by the owner; only they can unpin it, in the HQ app");
        }
        Ok(())
    }
}

/// The reorg tools, sharing one client and optional index DB.
pub fn create_vault_reorg_tools(
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
) -> Vec<Box<dyn HqTool>> {
    let db_for_frontmatter = db.clone();
    vec![
        Box::new(VaultMoveTool {
            vault: vault.clone(),
            db: db.clone(),
        }),
        Box::new(VaultDeleteTool {
            vault: vault.clone(),
            db,
        }),
        Box::new(VaultFrontmatterUpdateTool {
            vault: vault.clone(),
            db: db_for_frontmatter.clone(),
        }),
        Box::new(VaultPinTool {
            vault,
            db: db_for_frontmatter,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_agents_pin_is_a_suggestion_and_the_owners_pin_is_theirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        std::fs::write(
            dir.path().join("Notebooks/a.md"),
            "---\ntitle: A\n---\nbody",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Notebooks/mine.md"),
            "---\ntitle: Mine\npinned: true\n---\nbody",
        )
        .unwrap();
        let vault = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = VaultPinTool {
            vault: vault.clone(),
            db: Some(db.clone()),
        };
        let suggested = |rel: &str| {
            let raw = std::fs::read_to_string(dir.path().join(rel)).unwrap();
            db.with_conn(|c| hq_db::provenance::pin_suggested_by(c, rel, &raw))
                .unwrap()
        };

        tool.execute(json!({"path": "Notebooks/a.md"}))
            .await
            .unwrap();
        let raw = std::fs::read_to_string(dir.path().join("Notebooks/a.md")).unwrap();
        assert!(
            raw.contains("pinned: true") && raw.ends_with("body"),
            "{raw}"
        );
        assert_eq!(
            suggested("Notebooks/a.md").as_deref(),
            Some(UNATTRIBUTED_PINNER),
            "a pin through the tool is never the owner's"
        );
        tool.execute(json!({"path": "Notebooks/a.md", "pin": false}))
            .await
            .unwrap();
        assert!(
            !std::fs::read_to_string(dir.path().join("Notebooks/a.md"))
                .unwrap()
                .contains("pinned")
        );

        let err = tool
            .execute(json!({"path": "Notebooks/mine.md", "pin": false}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("pinned by the owner"), "{err}");
        assert_eq!(suggested("Notebooks/mine.md"), None);
    }

    #[test]
    fn moving_a_note_re_indexes_it_at_the_new_path_and_keeps_its_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        std::fs::write(dir.path().join("Notebooks/new.md"), "a note about herons").unwrap();
        let vault = VaultClient::new(dir.path().to_path_buf()).unwrap();
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            hq_db::search::index_note(c, "Notebooks/old.md", "old", "a note about herons", "")?;
            hq_db::search::add_graph_link(c, "Notebooks/old.md", "Notebooks/x.md", 0.9, "applied")?;
            Ok(())
        })
        .unwrap();

        reindex_moved_note(Some(&db), &vault, "Notebooks/old.md", "Notebooks/new.md");

        let (hits, related_new, related_old) = db
            .with_conn(|c| {
                Ok((
                    hq_db::search::keyword_search(c, "herons", 10)?,
                    hq_db::search::get_related_paths(c, "Notebooks/new.md", 10)?,
                    hq_db::search::get_related_paths(c, "Notebooks/old.md", 10)?,
                ))
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note_path, "Notebooks/new.md");
        assert_eq!(related_new, vec!["Notebooks/x.md"]);
        assert!(related_old.is_empty());
    }
}
