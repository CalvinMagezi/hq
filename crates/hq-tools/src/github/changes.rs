//! What changed in a tarball checkout since its base record, read safely enough to publish.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::tarball::{Base, BaseFile, is_git_dir};

const MIB: u64 = 1024 * 1024;
/// GitHub's secondary rate limits make one blob request per file the cost to watch.
const MAX_CHANGED_FILES: usize = 100;
const MAX_PUSH_FILE_BYTES: u64 = 5 * MIB;
/// Everything one push uploads, held in memory on a phone while it is sent.
const MAX_PUSH_TOTAL_BYTES: u64 = 25 * MIB;
const COPY_BUF: usize = 64 * 1024;
/// The unpack's record cap, so every checkout it makes can be pushed from.
const MAX_WALKED_FILES: usize = 200_000;
/// Workflows run with the repository's secrets before anyone reviews the pull request.
const WORKFLOW_DIR: &str = ".github";
const MODE_EXECUTABLE: &str = "100755";
const MODE_REGULAR: &str = "100644";

/// One file to publish: new or changed (`content` set) or deleted (`content` empty).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub path: String,
    pub mode: String,
    pub content: Option<Vec<u8>>,
}

/// Every file that differs from `base`. Refuses the whole set on a link, a hard link, a `.git`
/// or `.github` entry, a file too large, too many changes, or anything shaped like a credential
/// (or equal to one of `secrets`), so nothing is published by accident.
pub(super) fn changes(checkout: &Path, base: &Base, secrets: &[String]) -> Result<Vec<Change>> {
    let root = checked_root(checkout)?;
    let known: BTreeMap<&str, &BaseFile> =
        base.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut total = 0u64;
    for rel in regular_files(&root)? {
        let change = changed_file(&root, &rel, known.get(rel.as_str()).copied(), secrets)?;
        seen.insert(rel);
        let Some(change) = change else { continue };
        total += change.content.as_ref().map_or(0, |c| c.len() as u64);
        if total > MAX_PUSH_TOTAL_BYTES {
            bail!(
                "the changes add up to more than {} MiB; push smaller pieces of work",
                MAX_PUSH_TOTAL_BYTES / MIB
            );
        }
        out.push(change);
        check_count(out.len())?;
    }
    for (path, file) in &known {
        if !seen.contains(*path) {
            refuse_workflow(path)?;
            out.push(Change {
                path: path.to_string(),
                mode: file.mode.clone(),
                content: None,
            });
            check_count(out.len())?;
        }
    }
    Ok(out)
}

fn check_count(count: usize) -> Result<()> {
    if count > MAX_CHANGED_FILES {
        bail!("more than {MAX_CHANGED_FILES} files changed; push smaller pieces of work");
    }
    Ok(())
}

fn refuse_workflow(rel: &str) -> Result<()> {
    if rel.split('/').next() == Some(WORKFLOW_DIR) {
        bail!(
            "{rel} is under {WORKFLOW_DIR}/, which runs with the repository's secrets; it is never pushed"
        );
    }
    Ok(())
}

/// The checkout's canonical path, refusing a checkout that is itself a link.
fn checked_root(checkout: &Path) -> Result<PathBuf> {
    let meta = fs::symlink_metadata(checkout).context("the checkout is missing; fetch it again")?;
    if !meta.is_dir() {
        bail!("the checkout is not a plain folder");
    }
    Ok(checkout.canonicalize()?)
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
fn changed_file(
    root: &Path,
    rel: &str,
    base: Option<&BaseFile>,
    secrets: &[String],
) -> Result<Option<Change>> {
    let plain = read_plain(root, rel)?;
    let mode = if plain.executable {
        MODE_EXECUTABLE
    } else {
        MODE_REGULAR
    };
    if base.is_some_and(|b| b.sha256 == plain.sha256 && b.mode == mode) {
        return Ok(None);
    }
    refuse_workflow(rel)?;
    let Some(content) = plain.content else {
        bail!(
            "{rel} changed and is larger than {} MiB",
            MAX_PUSH_FILE_BYTES / MIB
        );
    };
    refuse_secrets(rel, &content, base.is_some_and(|b| b.flagged), secrets)?;
    Ok(Some(Change {
        path: rel.to_string(),
        mode: mode.to_string(),
        content: Some(content),
    }))
}

struct Plain {
    sha256: String,
    executable: bool,
    /// Kept only when small enough to push; a larger file is hashed and dropped as it is read.
    content: Option<Vec<u8>>,
}

/// A file that held a credential shape when fetched (a test fixture, an example env file) may
/// still be edited; any file is refused when it contains one of the live `secrets` verbatim.
fn refuse_secrets(
    rel: &str,
    content: &[u8],
    already_flagged: bool,
    secrets: &[String],
) -> Result<()> {
    let text = String::from_utf8_lossy(content);
    let leaked = secrets
        .iter()
        .any(|s| !s.is_empty() && text.contains(s.as_str()));
    let shaped = !already_flagged && hq_core::redact::looks_like_credential(&text);
    if leaked || shaped || hq_core::redact::looks_like_credential(rel) {
        bail!("{rel} holds something shaped like a credential; nothing was pushed");
    }
    Ok(())
}

/// Reads one checkout file without following a link at any step: the parent must still resolve
/// inside the checkout, and the file is opened with `O_NOFOLLOW` and checked on the open handle.
fn read_plain(root: &Path, rel: &str) -> Result<Plain> {
    let path = root.join(rel);
    let parent = path.parent().context("a checkout file has no parent")?;
    if !parent.canonicalize()?.starts_with(root) {
        bail!("{rel} now resolves outside the checkout; nothing was pushed");
    }
    let mut file = open_no_follow(&path)
        .with_context(|| format!("{rel} could not be opened as a plain file"))?;
    let meta = file.metadata()?;
    if !meta.is_file() || hard_links(&meta) > 1 {
        bail!("{rel} is not a plain file with a single name; only plain files are pushed");
    }
    let mut hasher = Sha256::new();
    let mut kept = Some(Vec::new());
    let mut buf = vec![0u8; COPY_BUF];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        kept = kept
            .filter(|k| (k.len() + n) as u64 <= MAX_PUSH_FILE_BYTES)
            .map(|mut k| {
                k.extend_from_slice(&buf[..n]);
                k
            });
    }
    Ok(Plain {
        sha256: hex::encode(hasher.finalize()),
        executable: is_executable(&meta),
        content: kept,
    })
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<fs::File> {
    fs::File::open(path)
}

#[cfg(unix)]
fn hard_links(meta: &fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(meta)
}

#[cfg(not(unix))]
fn hard_links(_meta: &fs::Metadata) -> u64 {
    1
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
