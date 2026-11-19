//! Which note paths the note tools may touch: inside the vault, outside the private folders and outside the folders HQ loads as instructions.

use anyhow::{Result, bail};
use hq_vault::rel_path::{DATA_FOLDER, INSTRUCTION_FOLDERS, PRIVATE_FOLDERS, RelPath};
use std::path::Path;

/// Reject paths that escape the vault root via `..` or absolute prefixes, and any path that
/// does not parse as one vault path (a backslash, a NUL byte).
pub(crate) fn validate_path(path: &str) -> Result<()> {
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
pub(crate) fn validate_path_in_vault(vault_path: &Path, path: &str) -> Result<()> {
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
pub(crate) fn top_folder_in<'a>(path: &str, folders: &[&'a str]) -> Option<&'a str> {
    match hq_vault::rel_path::protected_folder(path, folders) {
        Ok(found) => found,
        Err(_) => folders.first().copied(),
    }
}

/// Whether the first real component of `path` is a private folder, however it is spelled
/// (`./_system/x`, `_System/x` on a case-insensitive volume).
pub(crate) fn names_private_folder(path: &str) -> bool {
    top_folder_in(path, PRIVATE_FOLDERS).is_some()
}

/// Refuse a note write into HQ's private folders or into a folder whose files become
/// instructions. Skills, slash commands, agent definitions and mailboxes each have a managed
/// tool with its own approval, staging and scanning; a plain note write would skip all of it.
pub(crate) fn validate_write_path(vault_path: &Path, path: &str) -> Result<()> {
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
pub(crate) fn instruction_folder_hint(folder: &str) -> &'static str {
    match folder {
        "skills" | "_skills" => "use skill_manage to create or change a skill",
        "_commands" => "use slash_command_manage to create or change a slash command",
        "_mailboxes" => "use agent_send_message to leave a message",
        _ => "the owner edits agent definitions directly",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(path: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        validate_write_path(dir.path(), path)
            .expect_err(path)
            .to_string()
    }

    #[test]
    fn private_and_instruction_folders_are_refused_however_they_are_spelled() {
        for p in [
            "_system/SOUL.md",
            "./_system/x.md",
            "_System/x.md",
            "_threads/a.md",
        ] {
            assert!(refused(p).contains("direct note write rejected"), "{p}");
        }
        for p in ["skills/a.md", "_commands/a.md", "_mailboxes/relay/a.md"] {
            assert!(
                refused(p).contains("holds files HQ loads as instructions"),
                "{p}"
            );
        }
    }

    #[test]
    fn the_data_folder_and_traversal_are_refused() {
        assert!(refused("_data/vault.db").contains("keys and database"));
        assert!(refused("../outside.md").contains("path traversal"));
        assert!(refused("/etc/passwd").contains("path traversal"));
        assert!(refused("Notebooks/").contains("must point to a file"));
    }

    #[test]
    fn an_ordinary_note_path_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        validate_write_path(dir.path(), "Notebooks/Projects/plan.md").unwrap();
    }
}
