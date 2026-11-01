//! Fetching a repository with neither `git` nor `gh` (a phone): its tarball is untrusted input.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::{CLONE_TIMEOUT, USER_AGENT, read_request};

const MIB: u64 = 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 256 * MIB;
const MAX_UNPACKED_BYTES: u64 = 1024 * MIB;
/// Every tar record, so a repository whose files each need a name record still fits.
const MAX_RECORDS: usize = 200_000;
/// A header and its padding are at most two blocks.
const RECORD_OVERHEAD_BYTES: u64 = 1024;
/// Everything that comes out of the decompressor, skipped bodies and headers included.
const MAX_STREAM_BYTES: u64 = MAX_UNPACKED_BYTES + MAX_RECORDS as u64 * RECORD_OVERHEAD_BYTES;
/// A pax or long-name record is read whole, so its declared size is checked first.
const MAX_META_BYTES: u64 = 64 * 1024;

const MAX_PATH_PARTS: usize = 128;
const MAX_PATH_BYTES: usize = 4096;
const MAX_UNPACK_TIME: Duration = Duration::from_secs(300);
/// Leftovers of a fetch that was killed are swept once they are this old.
const STALE_AFTER: Duration = Duration::from_secs(3600);
const COMMIT_SHA_LEN: usize = 40;
const COPY_BUF: usize = 64 * 1024;
/// Files up to this size are scanned for credential shapes as they are written.
const MAX_SCANNED_BYTES: u64 = 5 * MIB;
const MODE_EXECUTABLE: &str = "100755";
const MODE_REGULAR: &str = "100644";
const PERM_EXECUTABLE: u32 = 0o755;
const PERM_REGULAR: u32 = 0o644;
const EXEC_BITS: u32 = 0o111;
/// How many skipped entries a reply names; the count it gives is always exact.
pub(super) const SKIPPED_SHOWN: usize = 20;
const BASE_RECORD_FILE: &str = "base.json";
const ARCHIVE_PREFIX: &str = ".hq-tarball-";
const STAGING_PREFIX: &str = ".hq-unpack-";
const RECORD_TMP_PREFIX: &str = ".hq-record-";
const LEFTOVER_PREFIXES: [&str; 3] = [ARCHIVE_PREFIX, STAGING_PREFIX, RECORD_TMP_PREFIX];
/// Asks the commits endpoint for the bare commit id instead of a JSON document.
const ACCEPT_SHA: &str = "application/vnd.github.sha";
const ACCEPT_TARBALL: &str = "application/vnd.github+json";

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub max_unpacked_bytes: u64,
    pub max_stream_bytes: u64,
    pub max_records: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_unpacked_bytes: MAX_UNPACKED_BYTES,
            max_stream_bytes: MAX_STREAM_BYTES,
            max_records: MAX_RECORDS,
        }
    }
}

/// One file as it was unpacked: what a later write compares against to find changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BaseFile {
    pub path: String,
    pub mode: String,
    pub sha256: String,
    pub size: u64,
    /// Already held a credential-shaped string when fetched, so an edit to it is not a new leak.
    #[serde(default)]
    pub flagged: bool,
}

/// The commit a checkout came from and every file it holds, kept beside the checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Base {
    pub repo: String,
    pub commit: String,
    pub files: Vec<BaseFile>,
}

#[derive(Debug, Default)]
pub(crate) struct Unpacked {
    pub files: Vec<BaseFile>,
    /// Entries left out on purpose (links, devices, `.git`, colliding or non-text names).
    pub skipped: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Fetched {
    pub base: Base,
    pub skipped: Vec<String>,
}

/// The base record in a repository's records folder, which file tools may not touch.
pub(crate) fn base_record_path(records: &Path) -> PathBuf {
    records.join(BASE_RECORD_FILE)
}

/// The base record in `records`, if one exists, parses and names a real commit id.
pub(crate) fn load_base(records: &Path) -> Option<Base> {
    let bytes = fs::read(base_record_path(records)).ok()?;
    let base: Base = serde_json::from_slice(&bytes).ok()?;
    let commit_ok =
        base.commit.len() == COMMIT_SHA_LEN && base.commit.bytes().all(|b| b.is_ascii_hexdigit());
    commit_ok.then_some(base)
}

/// Removes a path when dropped unless disarmed, so a cancelled fetch leaves nothing behind.
struct Leftover {
    path: PathBuf,
    armed: bool,
}

impl Leftover {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Leftover {
    fn drop(&mut self) {
        if self.armed && fs::remove_dir_all(&self.path).is_err() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Tells the blocking unpack to stop when the fetch that started it is dropped.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(CLONE_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()?)
}

/// Resolves the default branch's commit, downloads its tarball and unpacks it at `dest`, which
/// must not exist yet, recording the base in `records`. Both appear only when every entry passed.
pub(crate) async fn fetch(
    api_base: &str,
    repo: &str,
    dest: &Path,
    records: &Path,
) -> Result<Fetched> {
    let client = client()?;
    let commit = resolve_commit(&client, api_base, repo).await?;
    let parent = dest.parent().context("the clone folder has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    sweep_leftovers(parent);
    tokio::fs::create_dir_all(records).await?;
    sweep_leftovers(records);
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let archive = Leftover::new(parent.join(format!("{ARCHIVE_PREFIX}{nonce}.tar.gz")));
    let staging_path = parent.join(format!("{STAGING_PREFIX}{nonce}"));
    let mut record_tmp = Leftover::new(records.join(format!("{RECORD_TMP_PREFIX}{nonce}")));
    let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));

    let url = format!("{api_base}repos/{repo}/tarball/{commit}");
    download(read_request(&client, &url, ACCEPT_TARBALL), &archive.path).await?;
    let (unpacked, mut staging) =
        unpack_file(&archive.path, staging_path, Arc::clone(&stop.0)).await?;
    let base = Base {
        repo: repo.to_string(),
        commit,
        files: unpacked.files,
    };
    fs::write(&record_tmp.path, serde_json::to_vec_pretty(&base)?)?;
    fs::rename(&staging.path, dest)
        .with_context(|| format!("could not move the checkout to {}", dest.display()))?;
    staging.disarm();
    // Only now is `dest` known to be this fetch's, so an earlier record may be replaced.
    let record = base_record_path(records);
    if let Err(e) = fs::rename(&record_tmp.path, &record) {
        let _ = fs::remove_dir_all(dest);
        return Err(e).context("could not save the checkout's base record");
    }
    record_tmp.disarm();
    Ok(Fetched {
        base,
        skipped: unpacked.skipped,
    })
}

fn sweep_leftovers(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = LEFTOVER_PREFIXES.iter().any(|p| name.starts_with(p));
        let age = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok()?.elapsed().ok());
        if ours && age.is_some_and(|a| a > STALE_AFTER) {
            drop(Leftover::new(entry.path()));
        }
    }
}

async fn resolve_commit(client: &reqwest::Client, api_base: &str, repo: &str) -> Result<String> {
    let url = format!("{api_base}repos/{repo}/commits/HEAD");
    let resp = read_request(client, &url, ACCEPT_SHA).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        bail!("GitHub returned {status} resolving {repo}'s commit");
    }
    let sha = body.trim();
    if sha.len() != COMMIT_SHA_LEN || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("GitHub did not return a commit id for {repo}");
    }
    Ok(sha.to_ascii_lowercase())
}

async fn download(req: reqwest::RequestBuilder, to: &Path) -> Result<()> {
    let mut resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GitHub returned {status} for the tarball");
    }
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_DOWNLOAD_BYTES)
    {
        bail!(
            "the tarball is larger than {} MiB",
            MAX_DOWNLOAD_BYTES / MIB
        );
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .await?;
    let mut got = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        got = got.saturating_add(chunk.len() as u64);
        if got > MAX_DOWNLOAD_BYTES {
            bail!(
                "the tarball is larger than {} MiB",
                MAX_DOWNLOAD_BYTES / MIB
            );
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

/// The staging folder's guard lives on the blocking thread, so a fetch dropped mid-unpack has
/// its folder removed by that thread once it stops, never while it is still writing.
async fn unpack_file(
    archive: &Path,
    staging: PathBuf,
    stop: Arc<AtomicBool>,
) -> Result<(Unpacked, Leftover)> {
    let archive = archive.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let guard = Leftover::new(staging);
        if stop.load(Ordering::Relaxed) {
            bail!("the tarball unpack was stopped");
        }
        let file = File::open(&archive)?;
        fs::create_dir(&guard.path)?;
        let reader = GzDecoder::new(BufReader::new(file));
        let unpacked = unpack(reader, &guard.path, Limits::default(), &stop)?;
        Ok((unpacked, guard))
    })
    .await?
}

/// Fails the read once more than the cap has come out, instead of ending the stream quietly.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.left = self.left.checked_sub(n as u64).ok_or_else(|| {
            io::Error::other("the tarball decompresses to more than the allowed size")
        })?;
        Ok(n)
    }
}

#[derive(Default)]
struct State {
    top: Option<String>,
    /// A name from a pax or GNU long-name record, for the entry that follows it.
    pending_name: Option<Vec<u8>>,
    out: Unpacked,
    written: u64,
}

/// Unpacks a GitHub tarball's entries into `dest`, refusing the whole archive on any entry that
/// could land outside it. Raw mode: HQ, not the tar crate, reads every pax and long-name record.
pub(crate) fn unpack<R: Read>(
    reader: R,
    dest: &Path,
    limits: Limits,
    stop: &AtomicBool,
) -> Result<Unpacked> {
    let capped = Capped {
        inner: reader,
        left: limits.max_stream_bytes,
    };
    let mut archive = tar::Archive::new(capped);
    let deadline = Instant::now() + MAX_UNPACK_TIME;
    let mut state = State::default();
    let entries = archive
        .entries()
        .context("the tarball is not a tar archive")?;
    for (count, entry) in entries.raw(true).enumerate() {
        let mut entry = entry.context("the tarball is damaged")?;
        if count >= limits.max_records {
            bail!("the tarball holds more than {} records", limits.max_records);
        }
        if stop.load(Ordering::Relaxed) || Instant::now() > deadline {
            bail!("the tarball unpack was stopped or ran out of time");
        }
        if read_meta(&mut entry, &mut state.pending_name)? {
            continue;
        }
        let name = match state.pending_name.take() {
            Some(name) => name,
            None => entry.path_bytes().into_owned(),
        };
        place(&mut entry, &name, dest, &mut state, limits)?;
    }
    if state.pending_name.is_some() {
        bail!("the tarball ends with a name record that names nothing");
    }
    state.out.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(state.out)
}

/// Handles a record that describes the next entry instead of being one. Returns whether it was.
fn read_meta<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    pending: &mut Option<Vec<u8>>,
) -> Result<bool> {
    use tar::EntryType::{GNULongLink, GNULongName, XGlobalHeader, XHeader};
    let kind = entry.header().entry_type();
    if !matches!(kind, XGlobalHeader | XHeader | GNULongName | GNULongLink) {
        return Ok(false);
    }
    if entry.size() > MAX_META_BYTES {
        bail!("the tarball has an oversized {kind:?} record");
    }
    let names = matches!(kind, XHeader | GNULongName);
    if names && pending.is_some() {
        bail!("the tarball gives one entry two names");
    }
    let mut body = Vec::new();
    entry.read_to_end(&mut body)?;
    match kind {
        XHeader => *pending = pax_path(&body)?,
        GNULongName => {
            let end = body.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
            body.truncate(end);
            *pending = Some(body);
        }
        // The global header carries git's commit id; a long link names a link, never written.
        _ => {}
    }
    Ok(true)
}

/// The `path` of a pax record. A size override is refused: raw mode would not apply it, so the
/// archive would be read differently from how any tar tool reads it.
fn pax_path(body: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut path = None;
    for ext in tar::PaxExtensions::new(body) {
        let ext = ext.context("the tarball has a damaged pax record")?;
        match ext.key_bytes() {
            // The last `path` wins here; either way the name is checked before use.
            b"path" => path = Some(ext.value_bytes().to_vec()),
            b"size" => bail!("the tarball overrides a file size in a pax record"),
            _ => {}
        }
    }
    Ok(path)
}

fn place<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    name: &[u8],
    dest: &Path,
    state: &mut State,
    limits: Limits,
) -> Result<()> {
    let Ok(name) = std::str::from_utf8(name) else {
        let shown = String::from_utf8_lossy(name);
        let rel = shown.split_once('/').map_or(&*shown, |(_, rest)| rest);
        state.out.skipped.push(rel.to_string());
        return Ok(());
    };
    let parts = name_parts(name)?;
    let Some(rel) = inside_top(&parts, &mut state.top)? else {
        return Ok(());
    };
    if parts.iter().any(|p| is_git_dir(p)) {
        state.out.skipped.push(rel);
        return Ok(());
    }
    let kept = match entry.header().entry_type() {
        tar::EntryType::Directory => make_dirs(&dest.join(&rel))?,
        tar::EntryType::Regular => match write_file(entry, dest, &rel, &mut state.written, limits)?
        {
            Some(file) => {
                state.out.files.push(file);
                true
            }
            None => false,
        },
        _ => false,
    };
    if !kept {
        state.out.skipped.push(rel);
    }
    Ok(())
}

/// A name taken by another entry, which on a case-folding file system includes names that
/// differ only in case. The entry that came first stays; the later one is skipped.
fn is_collision(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::NotADirectory
    )
}

/// Creates a folder and its parents. `false` when a file already holds one of those names.
fn make_dirs(path: &Path) -> Result<bool> {
    match fs::create_dir_all(path) {
        Ok(()) => Ok(true),
        Err(e) if is_collision(&e) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("could not create {}", path.display())),
    }
}

fn forbidden_in_a_name(c: char) -> bool {
    c == '\0' || (cfg!(windows) && (c == '\\' || c == ':'))
}

/// Git's own folder under any spelling a file system treats as the same name.
pub(super) fn is_git_dir(part: &str) -> bool {
    let name = part.trim_end_matches(['.', ' ']).to_ascii_lowercase();
    name == ".git" || name == "git~1"
}

/// The entry's path as plain names. `..`, a root, a drive or a leading `.` refuses the archive.
fn name_parts(name: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    for component in Path::new(name).components() {
        let Component::Normal(part) = component else {
            bail!("the tarball has an entry that leaves its folder: {name}");
        };
        let part = part
            .to_str()
            .filter(|p| !p.is_empty() && !p.contains(forbidden_in_a_name))
            .context("the tarball has an entry whose name is not allowed")?;
        parts.push(part.to_string());
    }
    if parts.is_empty() || parts.len() > MAX_PATH_PARTS {
        bail!("the tarball has an entry with a path of unreasonable length");
    }
    Ok(parts)
}

/// GitHub wraps every entry in one `owner-repo-sha/` folder. Returns the path inside it, `None`
/// for the folder itself, and refuses an entry outside it.
fn inside_top(parts: &[String], top: &mut Option<String>) -> Result<Option<String>> {
    let first = &parts[0];
    match top {
        None => *top = Some(first.clone()),
        Some(t) if t != first => bail!("the tarball has entries outside its single top folder"),
        Some(_) => {}
    }
    if parts.len() == 1 {
        return Ok(None);
    }
    let rel = parts[1..].join("/");
    if rel.len() > MAX_PATH_BYTES {
        bail!("the tarball has an entry with a path of unreasonable length");
    }
    Ok(Some(rel))
}

/// Writes one file. `None` when its name collides with an earlier entry (see [`is_collision`]).
fn write_file<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    dest: &Path,
    rel: &str,
    written: &mut u64,
    limits: Limits,
) -> Result<Option<BaseFile>> {
    let target = dest.join(rel);
    if let Some(parent) = target.parent()
        && !make_dirs(parent)?
    {
        return Ok(None);
    }
    let opened = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target);
    let mut file = match opened {
        Ok(file) => file,
        Err(e) if is_collision(&e) => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("could not write {rel}")),
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUF];
    let mut size = 0u64;
    let mut scanned = Vec::new();
    loop {
        let n = entry.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size <= MAX_SCANNED_BYTES {
            scanned.extend_from_slice(&buf[..n]);
        }
        *written = written.saturating_add(n as u64);
        if *written > limits.max_unpacked_bytes {
            bail!(
                "the repository unpacks to more than {} MiB",
                limits.max_unpacked_bytes / MIB
            );
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    let executable = entry.header().mode().is_ok_and(|m| m & EXEC_BITS != 0);
    set_executable(&target, executable)?;
    Ok(Some(BaseFile {
        path: rel.to_string(),
        mode: if executable {
            MODE_EXECUTABLE
        } else {
            MODE_REGULAR
        }
        .to_string(),
        sha256: hex::encode(hasher.finalize()),
        size,
        flagged: hq_core::redact::looks_like_credential(&String::from_utf8_lossy(&scanned)),
    }))
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable {
        PERM_EXECUTABLE
    } else {
        PERM_REGULAR
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "tarball_tests.rs"]
mod tests;
