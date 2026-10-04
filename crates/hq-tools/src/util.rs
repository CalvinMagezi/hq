//! Shared utility functions used across hq-tools modules.

use serde_json::Value;
use std::path::Path;

/// Reads a string argument, defaulting to `""` when absent or not a string.
pub fn arg_str(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Reads an array-of-strings argument, skipping non-strings; empty when absent.
pub fn arg_str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Canonical-containment check: rejects `rel` if, once joined to `vault_path`,
/// it resolves outside the vault root (e.g. via a symlink). A not-yet-created
/// file has nothing on disk to canonicalize, so when the candidate can't be
/// canonicalized this compares both sides lexically instead — canonicalizing
/// only the vault root in that case would false-positive whenever the vault
/// itself sits under a symlinked prefix (e.g. macOS temp dirs under
/// `/var` -> `/private/var`), rejecting every not-yet-existing path. This is
/// defense-in-depth alongside (not a replacement for) string-level checks
/// like rejecting `..` or absolute paths.
pub fn assert_within_vault(vault_path: &Path, rel: &str) -> anyhow::Result<()> {
    let candidate = vault_path.join(rel);
    let (vault_cmp, cand_cmp) = match candidate.canonicalize() {
        Ok(cand_canon) => (
            vault_path
                .canonicalize()
                .unwrap_or_else(|_| vault_path.to_path_buf()),
            cand_canon,
        ),
        // Not-yet-existing target: nothing to canonicalize, so resolve `.`/`..`
        // lexically instead. `Path::starts_with` is a pure component-prefix
        // match with no `..` awareness — comparing the raw joined path would
        // let e.g. "../../etc/passwd" satisfy starts_with(vault_path) despite
        // actually resolving outside it.
        Err(_) => (
            lexically_normalize(vault_path),
            lexically_normalize(&candidate),
        ),
    };
    if !cand_cmp.starts_with(&vault_cmp) {
        anyhow::bail!("path escapes the vault: {}", rel);
    }
    Ok(())
}

/// Resolves `.`/`..` components without touching the filesystem (unlike
/// `canonicalize`, which requires the path to exist).
pub(crate) fn lexically_normalize(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Generate a unique ID with a customizable prefix.
///
/// Format: `{prefix}-{timestamp_ms}-{uuid_fragment}`
pub fn generate_id(prefix: &str) -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    let r = uuid::Uuid::new_v4().to_string();
    format!("{prefix}-{ts}-{}", &r[..6])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_within_vault_rejects_dotdot_traversal_to_a_not_yet_existing_target() {
        // No caller-side `.contains("..")` guard here on purpose: this proves
        // the function's own contract holds standalone, since `Path::starts_with`
        // does not resolve `..` and a naive lexical fallback would wrongly
        // accept this (candidate.canonicalize() fails because the target
        // doesn't exist, but the traversal still resolves outside the vault).
        let dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let outside_name = outside.path().file_name().unwrap().to_str().unwrap();
        let rel = format!("../{outside_name}/does-not-exist.md");
        assert!(assert_within_vault(dir.path(), &rel).is_err());
    }

    #[test]
    fn assert_within_vault_accepts_not_yet_existing_path_within_the_vault() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(assert_within_vault(dir.path(), "Notebooks/new-note.md").is_ok());
    }
}
