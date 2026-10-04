use anyhow::Result;
use hq_core::types::{Note, SystemContext};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{notes, system};

/// Main entry point for vault filesystem operations.
#[derive(Debug, Clone)]
pub struct VaultClient {
    vault_path: PathBuf,
}

impl VaultClient {
    pub fn new(vault_path: PathBuf) -> Result<Self> {
        if !vault_path.exists() {
            std::fs::create_dir_all(&vault_path)?;
        }
        Ok(Self { vault_path })
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    // --- Notes ---

    pub fn read_note(&self, rel_path: &str) -> Result<Note> {
        notes::read_note(&self.vault_path, rel_path)
    }

    /// Write a note. The path must end in `.md`; returns an error for any
    /// other extension to enforce the markdown-first vault invariant.
    pub fn write_note(&self, rel_path: &str, note: &Note) -> Result<()> {
        let path = std::path::Path::new(rel_path);
        if path.extension().map(|e| e != "md").unwrap_or(true) {
            anyhow::bail!(
                "vault write rejected: '{}' is not a markdown file (.md). \
                 Non-markdown files must be converted before entering the vault.",
                rel_path
            );
        }
        notes::write_note(&self.vault_path, rel_path, note)
    }

    pub fn list_notes(&self, dir: &str) -> Result<Vec<String>> {
        notes::list_notes(&self.vault_path, dir)
    }

    pub fn list_notes_recursive(&self, dir: &str) -> Result<Vec<String>> {
        notes::list_notes_recursive(&self.vault_path, dir)
    }

    pub fn note_exists(&self, rel_path: &str) -> bool {
        self.vault_path.join(rel_path).exists()
    }

    // --- Outline & Section Operations ---

    pub fn extract_outline(&self, rel_path: &str) -> Result<Vec<notes::NoteHeading>> {
        let note = self.read_note(rel_path)?;
        Ok(notes::extract_outline(&note.content))
    }

    pub fn read_section(
        &self,
        rel_path: &str,
        heading: &str,
    ) -> Result<Option<notes::NoteSection>> {
        let note = self.read_note(rel_path)?;
        Ok(notes::read_section(&note.content, heading))
    }

    pub fn append_to_note(&self, rel_path: &str, extra: &str, heading: Option<&str>) -> Result<()> {
        let mut note = self.read_note(rel_path)?;
        note.content = notes::append_to_note(&note.content, extra, heading);
        note.modified_at = chrono::Utc::now();
        self.write_note(rel_path, &note)
    }

    pub fn patch_section(&self, rel_path: &str, heading: &str, new_body: &str) -> Result<()> {
        let mut note = self.read_note(rel_path)?;
        note.content = notes::patch_section(&note.content, heading, new_body)?;
        note.modified_at = chrono::Utc::now();
        self.write_note(rel_path, &note)
    }

    pub fn extract_links(&self, rel_path: &str) -> Result<Vec<String>> {
        let note = self.read_note(rel_path)?;
        Ok(notes::extract_links(&note.content))
    }

    // --- Reorganization ---

    /// Move a note, rewriting inbound wikilinks. See `reorg::move_note`.
    pub fn move_note(
        &self,
        from: &str,
        to: &str,
        dry_run: bool,
    ) -> Result<crate::reorg::MoveReport> {
        crate::reorg::move_note(&self.vault_path, from, to, dry_run)
    }

    /// Move a note into `_trash/` (never a hard delete). Returns the trash path.
    pub fn trash_note(&self, rel_path: &str, dry_run: bool) -> Result<String> {
        crate::reorg::trash_note(&self.vault_path, rel_path, dry_run)
    }


    /// Set and/or remove frontmatter keys on a note, preserving the body.
    pub fn update_frontmatter(
        &self,
        rel_path: &str,
        set: HashMap<String, serde_yaml::Value>,
        remove: Vec<String>,
    ) -> Result<()> {
        crate::reorg::update_frontmatter(&self.vault_path, rel_path, set, remove)
    }

    // --- System Context ---

    pub fn get_system_context(&self) -> Result<SystemContext> {
        system::get_system_context(&self.vault_path)
    }

    pub fn read_system_file(&self, name: &str) -> Result<String> {
        system::read_system_file(&self.vault_path, name)
    }

    pub fn write_system_file(&self, name: &str, content: &str) -> Result<()> {
        system::write_system_file(&self.vault_path, name, content)
    }

    // --- Stats ---

    /// Returns (note_count, db_size_bytes).
    pub fn get_stats(&self) -> Result<(usize, u64)> {
        let all_notes = notes::list_notes_recursive(&self.vault_path, "")?;
        let db_path = self.vault_path.join("_data").join("vault.db");
        let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        Ok((all_notes.len(), db_size))
    }
}
