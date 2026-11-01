//! What changed in a tarball checkout since its base record, read safely enough to publish.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::tarball::{Base, is_git_dir};

/// GitHub's secondary rate limits make one blob request per file the cost to watch.
const MAX_CHANGED_FILES: usize = 100;
const MAX_PUSH_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// A checkout bigger than this is not one the worker should be pushing from.
const MAX_WALKED_FILES: usize = 100_000;
const MODE_EXECUTABLE: &str = "100755";
const MODE_REGULAR: &str = "100644";

/// One file to publish: new or changed (`content` set) or deleted (`content` empty).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub path: String,
    pub mode: String,
    pub content: Option<Vec<u8>>,
}

/// Every file that differs from `base`. Refuses the whole set on a link, a `.git` entry, a
/// file too large, too many changes, or anything shaped like a credential, so nothing is
/// published by accident. Only regular files under `checkout` are ever read.
pub(super) fn changes(checkout: &Path, base: &Base) -> Result<Vec<Change>> {
    let known: BTreeMap<&str, (&str, &str)> = base
        .files
        .iter()
        .map(|f| (f.path.as_str(), (f.sha256.as_str(), f.mode.as_str())))
        .collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for rel in regular_files(checkout)? {
        let change = changed_file(checkout, &rel, known.get(rel.as_str()).copied())?;
        seen.insert(rel);
        out.extend(change);
        if out.len() > MAX_CHANGED_FILES {
            bail!("more than {MAX_CHANGED_FILES} files changed; push smaller pieces of work");
        }
    }
    for (path, (_, mode)) in &known {
        if !seen.contains(*path) {
            out.push(Change {
                path: path.to_string(),
                mode: mode.to_string(),
                content: None,
            });
        }
    }
    if out.len() > MAX_CHANGED_FILES {
        bail!("more than {MAX_CHANGED_FILES} files changed; push smaller pieces of work");
    }
    Ok(out)
}

/// Paths of every regular file under `root`, relative and `/`-joined. Links are refused, never
/// followed: one pointing at a secret outside the checkout must not become a published file.
fn regular_files(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        let entries =
            fs::read_dir(root.join(&dir)).with_context(|| format!("cannot read {dir}"))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .context("the checkout holds a name that is not text")?;
            let rel = if dir.is_empty() {
                name.to_string()
            } else {
                format!("{dir}/{name}")
            };
            if is_git_dir(name) {
                bail!("the checkout holds {rel}; git metadata is never pushed");
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(rel);
            } else if kind.is_file() {
                files.push(rel);
            } else {
                bail!("{rel} is a link or special file; only plain files are pushed");
            }
            if files.len() > MAX_WALKED_FILES {
                bail!("the checkout holds more than {MAX_WALKED_FILES} files");
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The change for one file, or `None` when it matches the base exactly.
fn changed_file(root: &Path, rel: &str, base: Option<(&str, &str)>) -> Result<Option<Change>> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file() {
        bail!("{rel} changed into a link or special file while being read");
    }
    if meta.len() > MAX_PUSH_FILE_BYTES {
        bail!(
            "{rel} is larger than {} MiB",
            MAX_PUSH_FILE_BYTES / (1024 * 1024)
        );
    }
    let content = fs::read(&path)?;
    let mode = if is_executable(&meta) {
        MODE_EXECUTABLE
    } else {
        MODE_REGULAR
    };
    let sha = hex::encode(Sha256::digest(&content));
    if base.is_some_and(|(base_sha, base_mode)| base_sha == sha && base_mode == mode) {
        return Ok(None);
    }
    if hq_core::redact::looks_like_credential(&String::from_utf8_lossy(&content)) {
        bail!("{rel} holds something shaped like a credential; it was not pushed");
    }
    Ok(Some(Change {
        path: rel.to_string(),
        mode: mode.to_string(),
        content: Some(content),
    }))
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const EXEC_BITS: u32 = 0o111;
    meta.permissions().mode() & EXEC_BITS != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}
