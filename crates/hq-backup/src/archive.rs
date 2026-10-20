//! What goes into a backup and how it comes back out.
//!
//! The payload is a tar stream, compressed with gzip and sealed by `crypto`. Its entries are the
//! vault's files under `vault/`, a consistent snapshot of the database as `database/vault.db`,
//! optionally the config file as `config/config.yaml`, and last a `MANIFEST.json` that lists every
//! other entry with its size and SHA-256. Opening a backup reads the whole stream, which checks
//! every chunk, then checks the manifest against what was actually read, and only then is
//! anything put where the owner will look for it.
//!
//! A backup file is untrusted input when it is restored: entry names, types, sizes and counts are
//! all checked, and files are written only inside a fresh private folder next to the target.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{Header, OpenReader, SealWriter, classify, random};
use crate::error::{BackupError, Result, corrupt, refused};
use crate::fsutil;
use crate::key::{Argon2Cost, Secret};
use crate::{retention, snapshot};

pub const MANIFEST_FORMAT: u32 = 1;
const MANIFEST_NAME: &str = "MANIFEST.json";
const VAULT_DIR: &str = "vault";
const DB_ENTRY: &str = "database/vault.db";
const CONFIG_ENTRY: &str = "config/config.yaml";
/// Where the database lives inside a vault, and so where a restore puts it.
pub const DB_RELATIVE: &str = "_data/vault.db";
/// A manifest larger than this is not one.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;
const MAX_NAME_PARTS: usize = 64;
const MAX_NAME_BYTES: usize = 4096;
const COPY_BUF: usize = 64 * 1024;
const WORK_PREFIX: &str = "hq-backup-work-";
const RESTORE_PREFIX: &str = ".hq-restore-";
/// A work folder this old was left by a run that died.
const STALE_WORK: Duration = Duration::from_secs(24 * 60 * 60);
/// The report text for something the backup could not read; counted to tell a complete backup
/// from a hollow one.
const NOT_READABLE: &str = "not readable by this user";
/// Room kept free on top of what a backup is known to need in its work folder.
const WORK_MARGIN: u64 = 16 * 1024 * 1024;
/// A GNU long-name record is a path, and a path is at most this long. A bigger claim is not a name.
const MAX_EXTENSION_BYTES: u64 = 64 * 1024;
const TAR_BLOCK: usize = 512;
const WORK_NONCE_BYTES: usize = 8;
const RESTORE_NONCE_BYTES: usize = 6;
/// The folder of notes a person writes. Whatever is in it is theirs, so no rule about HQ's own
/// bookkeeping files applies there.
const NOTES_DIR: &str = "Notebooks";
/// Lock files HQ keeps in its own folders. They name a process that will not exist after a
/// restore, so a backup leaves them out.
const HQ_LOCK_NAMES: [&str; 4] = [".consolidate-lock", ".daemon-lock", "bot.lock", "sync.lock"];

/// One file in a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The path inside the archive, with `/` separators.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// What a backup holds, written into it as `MANIFEST.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub created: String,
    pub hq_version: String,
    /// The vault's files, by archive path (`vault/Notebooks/...`).
    pub files: Vec<Entry>,
    pub database: Entry,
    pub config: Option<Entry>,
    /// The vault's folders, relative to it, so that empty ones come back too: HQ expects some of
    /// them (`_mailboxes/relay` and the like) to exist. Absent in backups made before this field.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Things that were not backed up and why, such as links.
    pub skipped: Vec<String>,
}

impl Manifest {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum::<u64>()
            + self.database.size
            + self.config.as_ref().map_or(0, |c| c.size)
    }
}

/// What [`create`] needs.
pub struct BackupOptions<'a> {
    pub vault: &'a Path,
    /// The live database. Usually `<vault>/_data/vault.db`.
    pub db: &'a Path,
    /// The config file to include, if the owner asked for it. It holds API keys and tokens.
    pub config_file: Option<&'a Path>,
    /// A file to create, or an existing folder, in which a timestamped name is used.
    pub out: &'a Path,
    pub secret: &'a Secret,
    pub hq_version: &'a str,
    pub cost: Argon2Cost,
    /// Read the finished backup back and check it, which a backup nobody has opened is not.
    pub verify: bool,
}

#[derive(Debug, Clone)]
pub struct BackupReport {
    pub path: PathBuf,
    pub files: usize,
    /// Bytes before compression and encryption.
    pub bytes: u64,
    pub archive_bytes: u64,
    pub skipped: Vec<String>,
    /// How many of `skipped` are files or folders this user could not read. A backup missing
    /// those is not a complete copy, which a scheduled run must not let replace one that is.
    pub unreadable: usize,
    pub verified: bool,
}

/// What [`restore`] needs.
pub struct RestoreOptions<'a> {
    pub archive: &'a Path,
    /// A folder that does not exist yet, or an empty one. Anything else is refused.
    pub into: &'a Path,
    pub secret: &'a Secret,
    /// Where to put the config file if the backup has one. Without it the config is not restored.
    pub config_out: Option<&'a Path>,
    /// Rebuild the keyword index after restoring.
    pub reindex: bool,
    /// Refuse a backup that would unpack to more than this many bytes.
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub vault: PathBuf,
    pub manifest: Manifest,
    pub config_restored: Option<PathBuf>,
    pub warnings: Vec<String>,
}

/// Removes a folder on drop. Used for the work folders of a backup and a restore.
#[derive(Debug)]
struct Cleanup(PathBuf);

impl Cleanup {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Removes files on drop unless disarmed.
struct RemoveOnDrop {
    paths: Vec<PathBuf>,
    armed: bool,
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.armed {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}

struct HashRead<R> {
    inner: R,
    hasher: Sha256,
    count: u64,
}

impl<R: Read> Read for HashRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Opens a file inside the vault without following a link, so a file swapped for a link after the
/// walk is not read. Paths the owner names (the config, a backup to restore) are opened plainly:
/// a config kept in a dotfiles repository is usually a link.
fn open_no_follow(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn became_a_link(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        e.raw_os_error() == Some(libc::ELOOP)
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

/// A fresh private folder for plaintext that must not outlive the run: the database snapshot and
/// big files held for the stability check. The name is random and the folder is created
/// exclusively, so a name someone else planted is never reused. It goes in the system's temporary
/// folder, not beside the backup, so a backup folder that is synced or copied never holds
/// plaintext. Where that folder cannot be used (a phone's often cannot) `fallback` is used.
fn make_work_dir(fallback: Option<&Path>, need: u64) -> Result<Cleanup> {
    let parents: Vec<PathBuf> = std::iter::once(std::env::temp_dir())
        .chain(fallback.map(Path::to_path_buf))
        .collect();
    // Every candidate is swept, not only the one chosen: a folder left beside the backup by a run
    // that fell back to it must not stay there for as long as the temporary folder works.
    for parent in &parents {
        sweep_stale_work(parent);
    }
    let mut last = None;
    let mut roomy_enough = false;
    for parent in parents {
        // A small temporary folder (a RAM disk on many servers) is passed over, not run into.
        if free_bytes(&parent).is_some_and(|free| free < need) {
            continue;
        }
        roomy_enough = true;
        let name = format!(
            "{WORK_PREFIX}{}",
            hex::encode(random::<WORK_NONCE_BYTES>()?)
        );
        let path = parent.join(name);
        match fsutil::create_private_dir_new(&path) {
            Ok(()) => return Ok(Cleanup(path)),
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e.into()),
        None if !roomy_enough => refused(format!(
            "no folder has room for the {} MB the backup needs to work in; set TMPDIR to a folder \
             on a bigger disk",
            need / (1024 * 1024)
        )),
        None => refused("there is no folder to work in"),
    }
}

/// Bytes free for an ordinary user on the file system holding `path`, if it can be told.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: an all-zero statvfs is a valid value to hand to the call that fills it.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is NUL-terminated and outlives the call; `stats` is writable and sized.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Removes folders named `prefix` plus `nonce_hex` characters of hex, owned by the current user
/// and untouched for a day, that a run which died left in `parent`.
fn sweep_stale(parent: &Path, prefix: &str, nonce_hex: usize) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let named = entry.file_name().to_str().is_some_and(|n| {
            n.strip_prefix(prefix).is_some_and(|rest| {
                rest.len() == nonce_hex && rest.bytes().all(|b| b.is_ascii_hexdigit())
            })
        });
        let stale = entry.metadata().ok().is_some_and(|m| {
            m.is_dir()
                && fsutil::owned_by_me(&m)
                && m.modified()
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age >= STALE_WORK)
        });
        if named && stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn sweep_stale_work(parent: &Path) {
    sweep_stale(parent, WORK_PREFIX, WORK_NONCE_BYTES * 2);
}

/// The path with its longest existing part resolved, so a folder that does not exist yet can still
/// be compared with the vault before anything is created.
fn resolve_lenient(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut tail: Vec<&OsStr> = Vec::new();
    let mut cursor = absolute.as_path();
    loop {
        match fs::canonicalize(cursor) {
            Ok(base) => return Ok(tail.iter().rev().fold(base, |acc, part| acc.join(part))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (cursor.file_name(), cursor.parent()) else {
                    return refused(format!("cannot resolve {}", path.display()));
                };
                tail.push(name);
                cursor = parent;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Which files of a vault are not backed up: the database and its sidecars (a snapshot replaces
/// them), HQ's own lock files, logs, restore work folders and other backups.
struct Rules {
    /// Folder (relative to the vault) and file name of each database whose files are skipped.
    db_files: Vec<(PathBuf, String)>,
}

impl Rules {
    fn new(vault: &Path, db: &Path) -> Self {
        let mut db_files = vec![(PathBuf::from("_data"), "vault.db".to_string())];
        let name = db.file_name().and_then(OsStr::to_str);
        let dir = db.parent().and_then(|p| p.strip_prefix(vault).ok());
        if let (Some(dir), Some(name)) = (dir, name) {
            db_files.push((dir.to_path_buf(), name.to_string()));
        }
        Self { db_files }
    }

    fn skip_dir(&self, rel: &Path) -> bool {
        let name = rel.file_name().and_then(OsStr::to_str).unwrap_or("");
        rel == Path::new("_logs") || name.starts_with(RESTORE_PREFIX)
    }

    fn skip_file(&self, rel: &Path) -> bool {
        let name = rel.file_name().and_then(OsStr::to_str).unwrap_or("");
        if name.ends_with(&format!(".{}", retention::EXTENSION))
            || name.ends_with(&format!(".{}.partial", retention::EXTENSION))
            || is_hq_lock(rel, name)
        {
            return true;
        }
        self.db_files
            .iter()
            .any(|(dir, db)| rel.parent() == Some(dir.as_path()) && name.starts_with(db.as_str()))
    }
}

/// An HQ lock file outside the notes. A note folder can hold anything, `Cargo.lock` included, and
/// all of it is data.
fn is_hq_lock(rel: &Path, name: &str) -> bool {
    let in_notes = rel.components().next() == Some(Component::Normal(OsStr::new(NOTES_DIR)));
    let lock =
        HQ_LOCK_NAMES.contains(&name) || (name.starts_with(".pairing-") && name.ends_with(".lock"));
    lock && !in_notes
}

struct Found {
    path: PathBuf,
    archive_path: String,
    size: u64,
}

/// Why a vault path cannot be given an entry name.
enum Unnamable {
    NotText,
    TooLong,
}

fn archive_path(rel: &Path) -> std::result::Result<String, Unnamable> {
    let mut parts = vec![VAULT_DIR.to_string()];
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                parts.push(part.to_str().ok_or(Unnamable::NotText)?.to_string());
            }
            _ => return Err(Unnamable::NotText),
        }
    }
    let name = parts.join("/");
    // The limits a restore enforces, so a backup is never made that cannot be restored.
    if parts.len() > MAX_NAME_PARTS || name.len() > MAX_NAME_BYTES {
        return Err(Unnamable::TooLong);
    }
    Ok(name)
}

fn unnamable_note(why: &Unnamable, what: &str, shown: &str) -> String {
    match why {
        Unnamable::NotText => format!("{what} name is not valid text: {shown}"),
        Unnamable::TooLong => format!("{what} path is too long for a backup to hold: {shown}"),
    }
}

fn walk(
    root: &Path,
    rel: &Path,
    rules: &Rules,
    found: &mut Vec<Found>,
    dirs: &mut Vec<String>,
    skipped: &mut Vec<String>,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(root.join(rel))?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let child = rel.join(entry.file_name());
        let shown = child.display().to_string();
        let file_type = match entry.file_type() {
            Ok(kind) => kind,
            // Removed since the folder was listed.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if file_type.is_symlink() {
            skipped.push(format!("link not followed: {shown}"));
        } else if file_type.is_dir() {
            if !rules.skip_dir(&child) {
                match archive_path(&child) {
                    Ok(path) => dirs.push(path[VAULT_DIR.len() + 1..].to_string()),
                    Err(why) => {
                        skipped.push(unnamable_note(&why, "folder", &shown));
                        continue;
                    }
                }
                match walk(root, &child, rules, found, dirs, skipped) {
                    Err(BackupError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(BackupError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied => {
                        skipped.push(format!("folder {NOT_READABLE}: {shown}"));
                    }
                    other => other?,
                }
            }
        } else if file_type.is_file() {
            if rules.skip_file(&child) {
                continue;
            }
            match archive_path(&child) {
                Ok(archive_path) => found.push(Found {
                    path: entry.path(),
                    archive_path,
                    size: entry.metadata().map_or(0, |m| m.len()),
                }),
                Err(why) => skipped.push(unnamable_note(&why, "file", &shown)),
            }
        } else {
            skipped.push(format!("not a plain file: {shown}"));
        }
    }
    Ok(())
}

fn append_reader<W: Write, R: Read>(
    tar: &mut tar::Builder<W>,
    archive_path: &str,
    reader: R,
    size: u64,
    mtime: u64,
) -> Result<Entry> {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o600);
    header.set_mtime(mtime);
    header.set_entry_type(tar::EntryType::Regular);
    let mut hashed = HashRead {
        inner: reader.take(size),
        hasher: Sha256::new(),
        count: 0,
    };
    tar.append_data(&mut header, archive_path, &mut hashed)?;
    if hashed.count != size {
        return refused(format!(
            "{archive_path} changed while it was being backed up; run the backup again"
        ));
    }
    Ok(Entry {
        path: archive_path.to_string(),
        size,
        sha256: hex::encode(hashed.hasher.finalize()),
    })
}

/// Files up to this size are read into memory to be checked for stability; larger ones are staged
/// in the work folder.
const IN_MEMORY_LIMIT: u64 = 8 * 1024 * 1024;
/// A file modified more recently than this may be in the middle of being written, so it is taken
/// on a later round. Two identical looks at a file do not prove it is quiet: a writer that
/// truncates and then writes shows an empty file twice.
const QUIET_PERIOD: Duration = Duration::from_secs(1);
/// How many times the files that are not quiet are tried, and how long to wait between rounds, so
/// that a burst of fresh files costs a couple of seconds in total, not per file.
const SETTLE_ROUNDS: u32 = 4;
const SETTLE_PAUSE: Duration = Duration::from_millis(600);

enum Staged {
    Memory(Vec<u8>),
    Disk(PathBuf),
}

/// What became of one look at a file other programs may be writing.
enum Staging {
    /// Read whole from a file that had not been touched for a second and did not change while it
    /// was read: the copy, its length and its modification time.
    Ready(Staged, u64, u64),
    /// Modified too recently, or changed while it was read. Try again on a later round.
    Later,
    /// Gone, unreadable or no longer a plain file. It is left out and listed, not a reason to
    /// fail a backup of the rest.
    Left(String),
}

fn is_quiet(modified: Option<SystemTime>) -> bool {
    match modified {
        // No timestamp to judge by; the length checks below still apply.
        None => true,
        Some(at) => match SystemTime::now().duration_since(at) {
            Ok(age) => age >= QUIET_PERIOD,
            // A time in the future is a clock that is off, not a writer; waiting does not help.
            Err(_) => true,
        },
    }
}

fn stage_once(source: &Path, work: &Path, serial: usize) -> Result<Staging> {
    let mut file = match open_no_follow(source) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Staging::Left("removed while the backup ran".into()));
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Ok(Staging::Left(NOT_READABLE.into()));
        }
        Err(e) if became_a_link(&e) => {
            return Ok(Staging::Left(
                "replaced by a link while the backup ran".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let before = file.metadata()?;
    if !before.is_file() {
        return Ok(Staging::Left("no longer a plain file".into()));
    }
    let size = before.len();
    let modified = before.modified().ok();
    if !is_quiet(modified) {
        return Ok(Staging::Later);
    }
    let (staged, read) = if size <= IN_MEMORY_LIMIT {
        let mut bytes = Vec::with_capacity(size as usize);
        (&mut file)
            .take(IN_MEMORY_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        let read = bytes.len() as u64;
        (Staged::Memory(bytes), read)
    } else {
        let path = work.join(format!("stage-{serial}"));
        let _ = fs::remove_file(&path);
        let mut copy = fsutil::create_private_file(&path)?;
        let read = io::copy(&mut file, &mut copy)?;
        (Staged::Disk(path), read)
    };
    let after = file.metadata()?;
    if read == size && after.len() == size && after.modified().ok() == modified {
        return Ok(Staging::Ready(
            staged,
            size,
            modified.map_or(0, unix_seconds),
        ));
    }
    if let Staged::Disk(path) = staged {
        let _ = fs::remove_file(path);
    }
    Ok(Staging::Later)
}

fn append_staged<W: Write>(
    tar: &mut tar::Builder<W>,
    archive_path: &str,
    staged: Staged,
    size: u64,
    mtime: u64,
) -> Result<Entry> {
    match staged {
        Staged::Memory(bytes) => append_reader(tar, archive_path, &bytes[..], size, mtime),
        Staged::Disk(path) => {
            let entry = append_reader(tar, archive_path, File::open(&path)?, size, mtime);
            let _ = fs::remove_file(&path);
            entry
        }
    }
}

/// How many seconds ahead a timestamped name may step when another backup already took the
/// second: two backups in one second must both survive, and names must still sort by time.
const NAME_STEP_LIMIT: i64 = 120;

/// Whether `out` names a folder to put timestamped backups in: one that exists, or one that does
/// not yet and is not named like a backup file. (A fresh machine's `--out ~/backups` must make a
/// folder, not a file called `backups`.)
fn names_a_folder(out: &Path) -> bool {
    out.is_dir()
        || (fs::symlink_metadata(out).is_err()
            && out
                .extension()
                .is_none_or(|ext| ext != retention::EXTENSION))
}

fn resolve_out(out: &Path) -> Result<PathBuf> {
    let path = if names_a_folder(out) {
        let now = chrono::Utc::now();
        (0..NAME_STEP_LIMIT)
            .map(|step| out.join(retention::file_name(now + chrono::Duration::seconds(step))))
            .find(|candidate| fs::symlink_metadata(candidate).is_err())
            .ok_or_else(|| {
                BackupError::Refused(format!(
                    "{} already holds a backup for every second ahead",
                    out.display()
                ))
            })?
    } else {
        out.to_path_buf()
    };
    if fs::symlink_metadata(&path).is_ok() {
        return refused(format!(
            "{} already exists; backups are never overwritten",
            path.display()
        ));
    }
    Ok(path)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Makes a backup of a vault and its database. The finished file appears only when it is whole:
/// it is written under a `.partial` name and renamed, and with `verify` set it is read back first.
pub fn create(opts: &BackupOptions<'_>) -> Result<BackupReport> {
    let vault = fs::canonicalize(opts.vault)
        .map_err(|e| BackupError::Refused(format!("{}: {e}", opts.vault.display())))?;
    if !vault.is_dir() {
        return refused(format!("{} is not a folder", vault.display()));
    }
    let db = fs::canonicalize(opts.db)
        .map_err(|e| BackupError::Refused(format!("{}: {e}", opts.db.display())))?;
    // Opened now so that a missing config fails before any work is done.
    let config_source = opts
        .config_file
        .map(|path| {
            let file = File::open(path)
                .map_err(|e| BackupError::Refused(format!("{}: {e}", path.display())))?;
            let meta = file.metadata()?;
            if !meta.is_file() {
                return refused(format!("{} is not a plain file", path.display()));
            }
            Ok((file, meta))
        })
        .transpose()?;

    let out_path = resolve_out(opts.out)?;
    let out_parent = out_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    if resolve_lenient(&out_parent)?.starts_with(&vault) {
        return refused("write the backup outside the vault, which it would otherwise back up too");
    }
    fsutil::create_private_dir(&out_parent)?;
    let out_dir = fs::canonicalize(&out_parent)?;

    // Created exclusively before anything can remove it, so the guard only ever deletes a file
    // this run made. A second run that picks the same name fails here, not half way through.
    let partial = with_suffix(&out_path, ".partial");
    let file = match fsutil::create_private_file(&partial) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return refused(format!(
                "{} exists: another backup is being written there, or a dead one left it",
                partial.display()
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let mut partial_guard = RemoveOnDrop {
        paths: vec![partial.clone()],
        armed: true,
    };

    let rules = Rules::new(&vault, &db);
    let mut found = Vec::new();
    let mut dirs = Vec::new();
    let mut skipped = Vec::new();
    walk(
        &vault,
        Path::new(""),
        &rules,
        &mut found,
        &mut dirs,
        &mut skipped,
    )?;
    if found.len() + dirs.len() >= Limits::default().max_entries {
        return refused(
            "this vault holds more files and folders than a restore will unpack in one backup",
        );
    }
    // What the work folder must hold at once: the database snapshot and the biggest file that is
    // staged on disk rather than in memory.
    let biggest_staged = found
        .iter()
        .map(|f| f.size)
        .filter(|size| *size > IN_MEMORY_LIMIT)
        .max()
        .unwrap_or(0);
    let db_size = fs::metadata(&db)?.len();
    let work = make_work_dir(Some(&out_dir), db_size + biggest_staged + WORK_MARGIN)?;

    let snap = work.path().join("vault.db");
    snapshot::snapshot(&db, &snap)?;

    let header = Header::new(opts.secret.is_passphrase(), opts.cost)?;
    let key = opts.secret.derive(&header.kdf)?;
    let sealed = SealWriter::new(file, &key, &header)?;
    let mut tar = tar::Builder::new(GzEncoder::new(sealed, Compression::default()));

    let mut files = Vec::with_capacity(found.len());
    let mut pending: Vec<(usize, &Found)> = found.iter().enumerate().collect();
    for round in 0..SETTLE_ROUNDS {
        let mut later = Vec::new();
        for (serial, item) in pending {
            match stage_once(&item.path, work.path(), serial)? {
                Staging::Ready(staged, size, mtime) => files.push(append_staged(
                    &mut tar,
                    &item.archive_path,
                    staged,
                    size,
                    mtime,
                )?),
                Staging::Later => later.push((serial, item)),
                Staging::Left(why) => skipped.push(format!(
                    "{why}, left out: {}",
                    item.archive_path.trim_start_matches("vault/")
                )),
            }
        }
        pending = later;
        if pending.is_empty() {
            break;
        }
        if round + 1 < SETTLE_ROUNDS {
            std::thread::sleep(SETTLE_PAUSE);
        }
    }
    for (_, item) in pending {
        skipped.push(format!(
            "kept changing while it was being backed up, left out: {}",
            item.archive_path.trim_start_matches("vault/")
        ));
    }

    let snap_meta = fs::metadata(&snap)?;
    let database = append_reader(
        &mut tar,
        DB_ENTRY,
        File::open(&snap)?,
        snap_meta.len(),
        unix_seconds(SystemTime::now()),
    )?;
    // The snapshot and every staged file are in the archive now; the read-back below makes its own
    // work folder, and two copies of the database must not be on disk at once.
    drop(work);
    let config = match config_source {
        Some((file, meta)) => Some(append_reader(
            &mut tar,
            CONFIG_ENTRY,
            file,
            meta.len(),
            meta.modified().map_or(0, unix_seconds),
        )?),
        None => None,
    };

    let unreadable = skipped.iter().filter(|s| s.contains(NOT_READABLE)).count();
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        created: chrono::Utc::now().to_rfc3339(),
        hq_version: opts.hq_version.to_string(),
        files,
        database,
        config,
        dirs,
        skipped,
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| BackupError::Corrupt(format!("manifest: {e}")))?;
    if json.len() as u64 > MAX_MANIFEST_BYTES {
        return refused(
            "this vault has so many files that the list of them would be bigger than a restore \
             will read; it cannot be backed up as one file",
        );
    }
    append_reader(
        &mut tar,
        MANIFEST_NAME,
        &json[..],
        json.len() as u64,
        unix_seconds(SystemTime::now()),
    )?;

    let sealed = tar.into_inner()?.finish()?;
    let file = sealed.finish()?;
    file.sync_all()?;
    drop(file);
    let archive_bytes = fs::metadata(&partial)?.len();

    // Read back and checked under its unfinished name: a file with its final name is one that has
    // passed, so a kill during the check cannot leave an unchecked backup looking finished.
    if opts.verify {
        // Sized from what was just written, so a vault bigger than a restore's default ceiling
        // is not judged hostile by its own backup.
        let limits = Limits {
            max_entries: manifest.files.len() + manifest.dirs.len() + 2,
            max_bytes: manifest.total_bytes(),
        };
        let need = manifest.database.size + WORK_MARGIN;
        let checked = check_archive(&partial, opts.secret, &limits, false, need)?;
        if checked != manifest {
            return corrupt("the backup read back differently from what was written");
        }
    }
    publish(&partial, &out_path)?;
    partial_guard.armed = false;

    Ok(BackupReport {
        path: out_path,
        files: manifest.file_count(),
        bytes: manifest.total_bytes(),
        archive_bytes,
        skipped: manifest.skipped.clone(),
        unreadable,
        verified: opts.verify,
    })
}

/// Gives the finished file its name without ever replacing a file already there: a hard link fails
/// if the name is taken, where a rename would overwrite it. A file system without hard links gets
/// a rename after one more look.
fn publish(partial: &Path, out: &Path) -> Result<()> {
    match fs::hard_link(partial, out) {
        Ok(()) => {
            let _ = fs::remove_file(partial);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => refused(format!(
            "{} already exists; backups are never overwritten",
            out.display()
        )),
        Err(_) => {
            if fs::symlink_metadata(out).is_ok() {
                return refused(format!(
                    "{} already exists; backups are never overwritten",
                    out.display()
                ));
            }
            fs::rename(partial, out)?;
            Ok(())
        }
    }
}

/// How much a restore will unpack, at most.
struct Limits {
    max_entries: usize,
    max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 5_000_000,
            max_bytes: 64 * 1024 * 1024 * 1024,
        }
    }
}

impl Limits {
    fn with_max_bytes(max_bytes: Option<u64>) -> Self {
        let default = Self::default();
        Self {
            max_bytes: max_bytes.unwrap_or(default.max_bytes),
            ..default
        }
    }
}

/// Checks a folder path from the manifest the way entry names are checked: only plain names, no
/// `.` or `..`, nothing empty, nothing that could point outside the restore folder.
fn check_relative_dir(path: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('/').collect();
    let bad = path.is_empty()
        || path.len() > MAX_NAME_BYTES
        || parts.len() > MAX_NAME_PARTS
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(forbidden_in_a_name));
    if bad {
        return refused("the backup lists a folder with a path that is not allowed");
    }
    Ok(())
}

/// A character that cannot be part of a file name where this build runs. On Unix only NUL is
/// off limits: a colon or a backslash is an ordinary character in a note's title, and refusing it
/// would make a backup of a real vault impossible to restore. On Windows a colon names a drive or
/// a data stream and a backslash separates folders, so neither may hide inside a name.
fn forbidden_in_a_name(c: char) -> bool {
    c == '\0' || (cfg!(windows) && (c == '\\' || c == ':'))
}

/// Checks an entry name and turns it into the `/`-joined path it stands for. Anything that could
/// point outside the folder it is unpacked into is refused: absolute paths, `..`, drive letters,
/// backslashes, empty parts, and names that are not text.
fn entry_name<R: Read>(entry: &tar::Entry<'_, R>) -> Result<String> {
    let path = entry
        .path()
        .map_err(|e| BackupError::Corrupt(format!("an entry has an unreadable name: {e}")))?;
    let mut parts: Vec<&str> = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return refused("an entry in the backup has a path that leaves the restore folder");
        };
        let Some(part) = part.to_str() else {
            return refused("an entry in the backup has a name that is not text");
        };
        if part.is_empty() || part.contains(forbidden_in_a_name) {
            return refused(
                "an entry in the backup has a name with a character that is not allowed",
            );
        }
        parts.push(part);
    }
    if parts.is_empty() || parts.len() > MAX_NAME_PARTS {
        return refused("an entry in the backup has a path of an unreasonable length");
    }
    let name = parts.join("/");
    if name.len() > MAX_NAME_BYTES {
        return refused("an entry in the backup has a path of an unreasonable length");
    }
    Ok(name)
}

fn allowed_name(name: &str) -> bool {
    name == DB_ENTRY
        || name == CONFIG_ENTRY
        || name
            .strip_prefix("vault/")
            .is_some_and(|rest| !rest.is_empty())
}

fn read_manifest(
    entry: &mut impl Read,
    size: u64,
    failure: &dyn Fn(io::Error) -> BackupError,
) -> Result<Manifest> {
    if size > MAX_MANIFEST_BYTES {
        return refused("the manifest in the backup is too large");
    }
    let mut bytes = Vec::new();
    entry.take(size).read_to_end(&mut bytes).map_err(failure)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| BackupError::Corrupt(format!("the manifest cannot be read: {e}")))?;
    if manifest.format > MANIFEST_FORMAT {
        return Err(BackupError::Newer(format!(
            "this backup's manifest is format {}, which is newer than this HQ understands; update HQ",
            manifest.format
        )));
    }
    Ok(manifest)
}

/// Passes a decompressed tar stream through and refuses what tar-rs would otherwise buffer without
/// limit. It reads whole an extension record (a long name, PAX data) of whatever size its header
/// claims, and honours a PAX `size` that overrides the header's. HQ writes neither PAX nor sparse
/// entries and no path is longer than a few kilobytes, so a stream with a PAX or sparse header, or
/// a long-name record bigger than a path can be, is not one HQ made. The guard follows the
/// 512-byte block framing only as far as it takes to see each header, the same way tar-rs does
/// once PAX and sparse entries are out of the picture.
struct FrameGuard<R> {
    inner: R,
    header: [u8; TAR_BLOCK],
    filled: usize,
    /// Data and padding bytes still to pass before the next header.
    skip: u64,
    tripped: Rc<Cell<bool>>,
}

impl<R> FrameGuard<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            header: [0; TAR_BLOCK],
            filled: 0,
            skip: 0,
            tripped: Rc::new(Cell::new(false)),
        }
    }

    /// Set when the guard stopped the stream.
    fn tripped(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.tripped)
    }

    fn check_header(&mut self) -> io::Result<()> {
        let block: &[u8; TAR_BLOCK] = &self.header;
        let header = tar::Header::from_byte_slice(block);
        let kind = header.entry_type();
        let size = header.entry_size().unwrap_or(0);
        let refused = match kind {
            tar::EntryType::XHeader | tar::EntryType::XGlobalHeader | tar::EntryType::GNUSparse => {
                true
            }
            tar::EntryType::GNULongName | tar::EntryType::GNULongLink => size > MAX_EXTENSION_BYTES,
            _ => false,
        };
        if refused {
            self.tripped.set(true);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the backup holds a tar record HQ never writes",
            ));
        }
        self.skip = size
            .div_ceil(TAR_BLOCK as u64)
            .saturating_mul(TAR_BLOCK as u64);
        Ok(())
    }
}

impl<R: Read> Read for FrameGuard<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let mut data = &buf[..n];
        while !data.is_empty() {
            if self.skip > 0 {
                let take = self.skip.min(data.len() as u64) as usize;
                self.skip -= take as u64;
                data = &data[take..];
                continue;
            }
            let take = (TAR_BLOCK - self.filled).min(data.len());
            self.header[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == TAR_BLOCK {
                self.filled = 0;
                self.check_header()?;
            }
        }
        Ok(n)
    }
}

/// What [`extract`] writes while it reads.
#[derive(Clone, Copy)]
enum Unpack<'a> {
    /// Nothing: every entry is only read and hashed.
    #[cfg(test)]
    Nothing,
    /// The database entry, under this folder, so it can be checked as a database.
    Database(&'a Path),
    /// Every entry, under this folder.
    All(&'a Path),
}

impl<'a> Unpack<'a> {
    fn root_for(self, name: &str) -> Option<&'a Path> {
        match self {
            #[cfg(test)]
            Self::Nothing => None,
            Self::Database(root) => (name == DB_ENTRY).then_some(root),
            Self::All(root) => Some(root),
        }
    }
}

/// Reads a whole backup, checking everything, and writes what `unpack` asks for. The returned
/// manifest has been compared with what was actually read: same entries, same sizes, same hashes.
/// Nothing is trustworthy until this returns `Ok`.
fn extract(
    archive_path: &Path,
    secret: &Secret,
    unpack: Unpack<'_>,
    limits: &Limits,
) -> Result<Manifest> {
    let mut file = BufReader::new(File::open(archive_path).map_err(|e| {
        BackupError::Refused(format!("cannot open {}: {e}", archive_path.display()))
    })?);
    let (header, raw) = Header::read(&mut file)?;
    let key = secret.derive(&header.kdf)?;
    let reader = OpenReader::new(file, &key, &header, raw);
    let failure = reader.failure();
    let guard = FrameGuard::new(GzDecoder::new(reader));
    let tripped = guard.tripped();
    let fail = |e: io::Error| {
        if tripped.get() {
            return BackupError::Refused(
                "the backup holds a tar record that HQ never writes (an oversized name, or PAX or \
                 sparse data); it was not made by HQ"
                    .into(),
            );
        }
        classify(e, failure.get())
    };
    let mut archive = tar::Archive::new(guard);

    let mut seen: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut manifest: Option<Manifest> = None;
    let mut total = 0u64;
    let mut buf = vec![0u8; COPY_BUF];

    for entry in archive.entries().map_err(&fail)? {
        let mut entry = entry.map_err(&fail)?;
        let name = entry_name(&entry)?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            return refused(format!(
                "the backup holds something that is not a plain file ({name}); a backup made by \
                 HQ never does"
            ));
        }
        let size = entry.size();
        if name == MANIFEST_NAME {
            if manifest.is_some() {
                return refused("the backup holds two manifests");
            }
            manifest = Some(read_manifest(&mut entry, size, &fail)?);
            continue;
        }
        if !allowed_name(&name) {
            return refused(format!("the backup holds an entry it should not: {name}"));
        }
        if seen.len() >= limits.max_entries {
            return refused("the backup holds more files than a restore will unpack");
        }
        if seen.contains_key(&name) {
            return refused(format!("the backup holds {name} twice"));
        }
        total = total.saturating_add(size);
        if total > limits.max_bytes {
            return refused(
                "the backup would unpack to more than the allowed size; raise the limit if \
                 this is your own backup",
            );
        }

        let mut out = match unpack.root_for(&name) {
            Some(root) => {
                let target = root.join(&name);
                if let Some(parent) = target.parent() {
                    fsutil::create_private_dir(parent)?;
                }
                Some(fsutil::create_private_file(&target)?)
            }
            None => None,
        };
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        loop {
            let n = entry.read(&mut buf).map_err(&fail)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            if let Some(out) = out.as_mut() {
                out.write_all(&buf[..n])?;
            }
            written += n as u64;
        }
        if written != size {
            return corrupt(format!("{name} is shorter than its header says"));
        }
        if let Some(out) = out.as_mut() {
            let modified =
                UNIX_EPOCH.checked_add(Duration::from_secs(entry.header().mtime().unwrap_or(0)));
            if let Some(modified) = modified {
                let _ = out.set_modified(modified);
            }
        }
        seen.insert(name, (size, hex::encode(hasher.finalize())));
    }

    // Read to the very end, through gzip's checksum and the last sealed chunk: only then has the
    // whole backup been authenticated.
    let mut tail = archive.into_inner();
    io::copy(&mut tail, &mut io::sink()).map_err(&fail)?;

    let manifest =
        manifest.ok_or_else(|| BackupError::Corrupt("the backup has no manifest".into()))?;
    let mut expected: BTreeMap<&str, &Entry> = BTreeMap::new();
    for entry in manifest
        .files
        .iter()
        .chain([&manifest.database])
        .chain(manifest.config.as_ref())
    {
        if expected.insert(entry.path.as_str(), entry).is_some() {
            return corrupt(format!("the manifest lists {} twice", entry.path));
        }
    }
    if expected.len() != seen.len() {
        return corrupt("the files in the backup are not the files the manifest lists");
    }
    for (path, entry) in &expected {
        match seen.get(*path) {
            Some((size, sha256)) if *size == entry.size && *sha256 == entry.sha256 => {}
            Some(_) => return corrupt(format!("{path} does not match its recorded hash")),
            None => return corrupt(format!("{path} is listed but missing")),
        }
    }
    if manifest.database.path != DB_ENTRY {
        return corrupt("the database entry is not where it should be");
    }
    if manifest.dirs.len() > limits.max_entries.max(manifest.files.len()) {
        return refused("the backup lists more folders than a restore will create");
    }
    for dir in &manifest.dirs {
        check_relative_dir(dir)?;
    }
    Ok(manifest)
}

/// The checks that need the database as a file. With `require_known_schema` a database from a
/// newer HQ is refused, which a restore must; reading back a backup just made must not fail only
/// because the database being protected is newer than this build.
fn check_database(db: &Path, manifest: &Manifest, require_known_schema: bool) -> Result<()> {
    if manifest
        .files
        .iter()
        .any(|f| f.path.starts_with("vault/_data/vault.db"))
    {
        return refused("the backup holds a second database inside its files");
    }
    snapshot::check_integrity(db)?;
    if require_known_schema {
        let newer = snapshot::newer_migrations(db)?;
        if !newer.is_empty() {
            return Err(BackupError::Newer(format!(
                "this backup's database is from a newer HQ (it has {}); update HQ before \
                 restoring it",
                newer.join(", ")
            )));
        }
    }
    Ok(())
}

/// Reads all of a backup and checks its database too, writing nothing but the database into a
/// private work folder that is removed again.
fn check_archive(
    archive: &Path,
    secret: &Secret,
    limits: &Limits,
    require_known_schema: bool,
    need: u64,
) -> Result<Manifest> {
    let work = make_work_dir(archive.parent(), need)?;
    let manifest = extract(archive, secret, Unpack::Database(work.path()), limits)?;
    check_database(&work.path().join(DB_ENTRY), &manifest, require_known_schema)?;
    Ok(manifest)
}

/// Opens a backup and checks all of it, database included, without restoring anything. Returns
/// what it holds.
pub fn verify(archive: &Path, secret: &Secret) -> Result<Manifest> {
    verify_limited(archive, secret, None)
}

/// [`verify`] with a ceiling other than the default 64 GiB on what the backup may unpack to.
pub fn verify_limited(archive: &Path, secret: &Secret, max_bytes: Option<u64>) -> Result<Manifest> {
    // The database's size is not known before it is read, so the work folder is not sized.
    check_archive(archive, secret, &Limits::with_max_bytes(max_bytes), true, 0)
}

fn is_empty_dir(path: &Path) -> io::Result<bool> {
    Ok(fs::read_dir(path)?.next().is_none())
}

/// Whether `into` is an existing empty folder (true), a name that does not exist yet (false), or
/// something a restore must not go into (an error).
fn check_target(into: &Path) -> Result<bool> {
    match fs::symlink_metadata(into) {
        Ok(meta) if meta.file_type().is_dir() => {
            if !is_empty_dir(into)? {
                let left = fs::read_dir(into)?
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(RESTORE_PREFIX))
                    .count();
                let hint = if left > 0 {
                    format!(
                        " (it holds {left} folder(s) named {RESTORE_PREFIX}..., left by a restore that was interrupted; delete them)"
                    )
                } else {
                    String::new()
                };
                return refused(format!(
                    "{} is not empty{hint}; restore into a new or an empty folder",
                    into.display()
                ));
            }
            Ok(true)
        }
        Ok(_) => refused(format!("{} exists and is not a folder", into.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Puts the finished vault in an existing empty folder, which may be a mount point or the current
/// folder and so cannot be swapped for another: its contents are moved in. The work folder is
/// inside it, so every move stays on one filesystem.
fn move_into_existing(vault: &Path, into: &Path, work: &Path) -> Result<()> {
    for entry in fs::read_dir(into)? {
        if entry?.path() != work {
            return refused(format!(
                "{} changed while the restore ran; it was not filled",
                into.display()
            ));
        }
    }
    for entry in fs::read_dir(vault)? {
        let entry = entry?;
        let to = into.join(entry.file_name());
        fs::rename(entry.path(), &to).map_err(|e| {
            BackupError::Refused(format!(
                "could not move {} into {}: {e}. The restore stopped part way: empty {} and \
                 run it again",
                entry.path().display(),
                into.display(),
                into.display()
            ))
        })?;
    }
    Ok(())
}

/// Writes the config to a path that must not exist, readable only by its owner.
fn place_config(staged: &Path, out: &Path) -> io::Result<()> {
    if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fsutil::create_private_dir(dir)?;
    }
    let mut dest = fsutil::create_private_file(out)?;
    let result = io::copy(&mut File::open(staged)?, &mut dest).and_then(|_| dest.sync_all());
    if result.is_err() {
        let _ = fs::remove_file(out);
    }
    result
}

/// Restores a backup into a new or empty folder. Everything is unpacked and checked in a private
/// work folder first; then the vault appears, whole, in one rename (or, for a folder that already
/// exists, is moved in once all of it has checked out). A backup from a newer HQ, one that fails
/// its checks, or a wrong key leaves the target untouched.
pub fn restore(opts: &RestoreOptions<'_>) -> Result<RestoreReport> {
    let into = std::path::absolute(opts.into)?;
    // What an interrupted restore left a day ago is cleared first, so it cannot make this target
    // look occupied. Anything younger is left: it may be another restore still running.
    sweep_stale(&into, RESTORE_PREFIX, RESTORE_NONCE_BYTES * 2);
    let existing = check_target(&into)?;
    if let Some(out) = opts.config_out
        && fs::symlink_metadata(out).is_ok()
    {
        return refused(format!(
            "{} already exists; it was not replaced",
            out.display()
        ));
    }
    let work_parent = if existing {
        into.clone()
    } else {
        let Some(parent) = into.parent() else {
            return refused("cannot restore into the top of the filesystem");
        };
        fsutil::create_private_dir(parent)?;
        sweep_stale(parent, RESTORE_PREFIX, RESTORE_NONCE_BYTES * 2);
        parent.to_path_buf()
    };
    let nonce = hex::encode(random::<RESTORE_NONCE_BYTES>()?);
    let work = work_parent.join(format!("{RESTORE_PREFIX}{nonce}"));
    fsutil::create_private_dir_new(&work)?;
    let work_guard = Cleanup(work.clone());

    let limits = Limits::with_max_bytes(opts.max_bytes);
    let manifest = extract(opts.archive, opts.secret, Unpack::All(&work), &limits)?;

    let staged_db = work.join(DB_ENTRY);
    check_database(&staged_db, &manifest, true)?;

    let vault = work.join(VAULT_DIR);
    fsutil::create_private_dir(&vault)?;
    for dir in &manifest.dirs {
        fsutil::create_private_dir(&vault.join(dir))?;
    }
    fsutil::create_private_dir(&vault.join("_data"))?;
    fs::rename(&staged_db, vault.join(DB_RELATIVE))?;

    if existing {
        move_into_existing(&vault, &into, &work)?;
    } else {
        // The target may have been created while the work folder was filling.
        if fs::symlink_metadata(&into).is_ok() {
            return refused(format!(
                "{} appeared while the restore ran; it was not replaced",
                into.display()
            ));
        }
        fs::rename(&vault, &into)?;
    }

    // From here the vault is in place; nothing below may undo it.
    let mut warnings = Vec::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&into, fs::Permissions::from_mode(0o700)) {
            warnings.push(format!(
                "could not make {} private to you ({e}); check who can read it",
                into.display()
            ));
        }
    }
    let mut config_restored = None;
    if manifest.config.is_some() {
        match opts.config_out {
            Some(out) => match place_config(&work.join(CONFIG_ENTRY), out) {
                Ok(()) => config_restored = Some(out.to_path_buf()),
                Err(e) => warnings.push(format!(
                    "the vault is restored but the config could not be written to {} ({e}); \
                     restore the backup again into a scratch folder with --config-out to get it",
                    out.display()
                )),
            },
            None => warnings.push(
                "the backup holds a config file with keys and tokens; it was not restored. \
                 Pass a path for it to restore it."
                    .to_string(),
            ),
        }
    }
    drop(work_guard);

    if opts.reindex
        && let Err(e) = reindex(&into)
    {
        warnings.push(format!(
            "the restore is complete but the search index could not be rebuilt ({e}); run \
             `hq reindex`"
        ));
    }

    Ok(RestoreReport {
        vault: into,
        manifest,
        config_restored,
        warnings,
    })
}

fn reindex(vault: &Path) -> Result<()> {
    let db = hq_db::Database::open(&vault.join(DB_RELATIVE))
        .map_err(|e| BackupError::Database(e.to_string()))?;
    let notebooks = vault.join("Notebooks");
    if !notebooks.is_dir() {
        return Ok(());
    }
    db.with_conn(|conn| hq_db::search::rebuild_index(conn, &notebooks))
        .map_err(|e| BackupError::Database(e.to_string()))?;
    Ok(())
}

/// A backup file is untrusted input. These build archives by hand, with the real container and
/// real sealing but entries no HQ would write, and check each is refused with nothing written.
#[cfg(test)]
mod hostile {
    use super::*;

    const PASS: &str = "correct horse battery staple";
    const COST: Argon2Cost = Argon2Cost {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };

    type Tar = tar::Builder<GzEncoder<SealWriter<File>>>;

    fn secret() -> Secret {
        Secret::passphrase(PASS).unwrap()
    }

    fn db_bytes(dir: &Path) -> Vec<u8> {
        let live = dir.join("seed.db");
        drop(hq_db::Database::open(&live).unwrap());
        let copy = dir.join("seed-copy.db");
        snapshot::snapshot(&live, &copy).unwrap();
        fs::read(copy).unwrap()
    }

    fn seal(dir: &Path, name: &str, fill: impl FnOnce(&mut Tar)) -> PathBuf {
        let secret = secret();
        let header = Header::new(true, COST).unwrap();
        let key = secret.derive(&header.kdf).unwrap();
        let path = dir.join(name);
        let file = File::create(&path).unwrap();
        let sealed = SealWriter::new(file, &key, &header).unwrap();
        let mut tar = tar::Builder::new(GzEncoder::new(sealed, Compression::default()));
        fill(&mut tar);
        tar.into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .finish()
            .unwrap();
        path
    }

    /// Appends an entry with exactly this name, bypassing the checks the tar crate applies to
    /// names, so a hostile name can be written.
    fn raw(tar: &mut Tar, name: &[u8], kind: tar::EntryType, data: &[u8], link: Option<&[u8]>) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(kind);
        {
            let old = header.as_old_mut();
            old.name.fill(0);
            old.name[..name.len()].copy_from_slice(name);
            if let Some(link) = link {
                old.linkname[..link.len()].copy_from_slice(link);
            }
        }
        header.set_cksum();
        tar.append(&header, data).unwrap();
    }

    fn entry(path: &str, data: &[u8]) -> Entry {
        Entry {
            path: path.to_string(),
            size: data.len() as u64,
            sha256: hex::encode(Sha256::digest(data)),
        }
    }

    fn manifest(files: &[(&str, &[u8])], db: &[u8]) -> Manifest {
        Manifest {
            format: MANIFEST_FORMAT,
            created: "test".into(),
            hq_version: "test".into(),
            files: files.iter().map(|(p, d)| entry(p, d)).collect(),
            database: entry(DB_ENTRY, db),
            config: None,
            dirs: Vec::new(),
            skipped: Vec::new(),
        }
    }

    fn file(tar: &mut Tar, path: &str, data: &[u8]) {
        raw(tar, path.as_bytes(), tar::EntryType::Regular, data, None);
    }

    fn manifest_entry(tar: &mut Tar, manifest: &Manifest) {
        file(tar, MANIFEST_NAME, &serde_json::to_vec(manifest).unwrap());
    }

    /// A complete, valid archive: the given files, the database and a matching manifest.
    fn good(tar: &mut Tar, files: &[(&str, &[u8])], db: &[u8]) {
        for (path, data) in files {
            file(tar, path, data);
        }
        file(tar, DB_ENTRY, db);
        manifest_entry(tar, &manifest(files, db));
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".hq-restore-"))
            .collect()
    }

    /// Restores `archive` and checks the failure was clean: no target, no work folder left, and
    /// nothing created next to the target or above it.
    fn refuse(dir: &Path, archive: &Path) -> BackupError {
        let into = dir.join("restore").join("into");
        let err = restore(&RestoreOptions {
            archive,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: None,
        })
        .expect_err("the archive must be refused");
        assert!(!into.exists(), "a refused restore left a target behind");
        assert!(
            leftovers(&dir.join("restore")).is_empty(),
            "a work folder was left"
        );
        for name in ["evil.txt", "evil", "b", "pwn"] {
            assert!(
                !dir.join(name).exists() && !dir.join("restore").join(name).exists(),
                "{name}"
            );
        }
        err
    }

    #[test]
    fn a_hand_built_archive_restores_so_the_refusals_below_mean_something() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let path = seal(dir.path(), "ok.hqbak", |t| {
            good(t, &[("vault/Notebooks/a.md", b"hello")], &db);
        });
        let into = dir.path().join("restored");
        restore(&RestoreOptions {
            archive: &path,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: None,
        })
        .unwrap();
        assert_eq!(fs::read(into.join("Notebooks/a.md")).unwrap(), b"hello");
        assert!(into.join(DB_RELATIVE).is_file());
    }

    #[test]
    fn names_that_could_leave_the_restore_folder_are_refused_for_that_reason() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        for name in [
            "vault/../../evil.txt",
            "../evil.txt",
            "/etc/evil",
            "vault/a/../../b",
            "vault/..",
            "./vault/a.md",
        ] {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                raw(t, name.as_bytes(), tar::EntryType::Regular, b"x", None);
                good(t, &[], &db);
            });
            let err = refuse(dir.path(), &path);
            let reason = err.to_string();
            assert!(
                matches!(err, BackupError::Refused(_))
                    && (reason.contains("leaves the restore folder")
                        || reason.contains("character that is not allowed")),
                "{name:?} was refused for the wrong reason: {reason}"
            );
        }
    }

    /// Names that collapse to an ordinary one (an empty part, a NUL that ends the name) are not an
    /// escape, and the archive is still refused because the manifest does not list the entry.
    #[test]
    fn names_that_collapse_to_an_ordinary_name_do_not_pass_unlisted() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        for name in ["vault//a", "vault/a\0b", "vault/a\\b", "vault/C:/pwn"] {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                raw(t, name.as_bytes(), tar::EntryType::Regular, b"x", None);
                good(t, &[], &db);
            });
            assert!(
                matches!(refuse(dir.path(), &path), BackupError::Corrupt(_)),
                "{name:?}"
            );
        }
    }

    #[test]
    fn links_devices_and_folders_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Directory,
            tar::EntryType::Char,
            tar::EntryType::Block,
            tar::EntryType::Fifo,
        ] {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                raw(t, b"vault/thing", kind, b"", Some(b"/etc/passwd"));
                good(t, &[], &db);
            });
            let err = refuse(dir.path(), &path);
            assert!(matches!(err, BackupError::Refused(_)), "{kind:?}: {err}");
        }
    }

    #[test]
    fn duplicate_entries_unknown_prefixes_and_a_second_database_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        type Case<'a> = (&'a str, Vec<(&'a str, &'a [u8])>);
        let cases: Vec<Case> = vec![
            ("twice", vec![("vault/a.md", b"1"), ("vault/a.md", b"2")]),
            ("prefix", vec![("etc/passwd", b"x")]),
            ("bare vault", vec![("vault", b"x")]),
            (
                "second database",
                vec![("vault/_data/vault.db", b"not a database")],
            ),
            (
                "second database sidecar",
                vec![("vault/_data/vault.db-wal", b"x")],
            ),
        ];
        for (label, files) in cases {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                for (p, d) in &files {
                    file(t, p, d);
                }
                file(t, DB_ENTRY, &db);
                manifest_entry(t, &manifest(&files, &db));
            });
            let err = refuse(dir.path(), &path);
            assert!(matches!(err, BackupError::Refused(_)), "{label}: {err}");
        }
    }

    #[test]
    fn a_manifest_that_does_not_describe_the_files_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let files: &[(&str, &[u8])] = &[("vault/a.md", b"hello")];
        let good_manifest = manifest(files, &db);

        let mut wrong_hash = good_manifest.clone();
        wrong_hash.files[0].sha256 = hex::encode(Sha256::digest(b"something else"));
        let mut wrong_size = good_manifest.clone();
        wrong_size.files[0].size += 1;
        let mut lists_missing = good_manifest.clone();
        lists_missing.files.push(entry("vault/missing.md", b"x"));
        let mut hides_a_file = good_manifest.clone();
        hides_a_file.files.clear();
        let mut lists_twice = good_manifest.clone();
        lists_twice.files.push(lists_twice.files[0].clone());
        let mut wrong_db = good_manifest.clone();
        wrong_db.database.sha256 = hex::encode(Sha256::digest(b"another database"));

        for (label, bad) in [
            ("hash", wrong_hash),
            ("size", wrong_size),
            ("lists a missing file", lists_missing),
            ("hides a file", hides_a_file),
            ("lists a file twice", lists_twice),
            ("database hash", wrong_db),
        ] {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                file(t, "vault/a.md", b"hello");
                file(t, DB_ENTRY, &db);
                manifest_entry(t, &bad);
            });
            let err = refuse(dir.path(), &path);
            assert!(matches!(err, BackupError::Corrupt(_)), "{label}: {err}");
        }

        // No manifest, two manifests, a manifest that is not JSON, and one from a newer HQ.
        let path = seal(dir.path(), "bad.hqbak", |t| {
            file(t, "vault/a.md", b"hello");
            file(t, DB_ENTRY, &db);
        });
        assert!(matches!(refuse(dir.path(), &path), BackupError::Corrupt(_)));
        let path = seal(dir.path(), "bad.hqbak", |t| {
            good(t, files, &db);
            manifest_entry(t, &good_manifest);
        });
        assert!(matches!(refuse(dir.path(), &path), BackupError::Refused(_)));
        let path = seal(dir.path(), "bad.hqbak", |t| {
            file(t, "vault/a.md", b"hello");
            file(t, DB_ENTRY, &db);
            file(t, MANIFEST_NAME, b"{ not json");
        });
        assert!(matches!(refuse(dir.path(), &path), BackupError::Corrupt(_)));
        let mut newer = good_manifest;
        newer.format = MANIFEST_FORMAT + 1;
        let path = seal(dir.path(), "bad.hqbak", |t| {
            file(t, "vault/a.md", b"hello");
            file(t, DB_ENTRY, &db);
            manifest_entry(t, &newer);
        });
        assert!(matches!(refuse(dir.path(), &path), BackupError::Newer(_)));
    }

    #[test]
    fn folders_in_the_manifest_cannot_point_outside_the_restore_folder() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        for bad in [
            "../escape",
            "/abs",
            "a/../../b",
            "a//b",
            ".",
            "..",
            "a/./b",
            "",
        ] {
            let mut m = manifest(&[], &db);
            m.dirs = vec![bad.to_string()];
            let path = seal(dir.path(), "bad.hqbak", |t| {
                file(t, DB_ENTRY, &db);
                manifest_entry(t, &m);
            });
            let err = refuse(dir.path(), &path);
            assert!(matches!(err, BackupError::Refused(_)), "{bad:?}: {err}");
        }
        // Ordinary folders, including empty ones, are created.
        let mut m = manifest(&[], &db);
        m.dirs = vec![
            "Notebooks".into(),
            "Notebooks/Inbox".into(),
            "_mailboxes/relay".into(),
        ];
        let path = seal(dir.path(), "ok.hqbak", |t| {
            file(t, DB_ENTRY, &db);
            manifest_entry(t, &m);
        });
        let into = dir.path().join("restored");
        restore(&RestoreOptions {
            archive: &path,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: None,
        })
        .unwrap();
        assert!(into.join("Notebooks/Inbox").is_dir() && into.join("_mailboxes/relay").is_dir());
    }

    #[test]
    fn a_database_that_is_not_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let junk = b"this is not a SQLite database at all, only text".to_vec();
        let path = seal(dir.path(), "bad.hqbak", |t| {
            good(t, &[("vault/a.md", b"hello")], &junk)
        });
        assert!(refuse(dir.path(), &path).to_string().len() > 5);
    }

    #[test]
    fn the_entry_and_size_limits_hold_while_reading() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let files: &[(&str, &[u8])] = &[("vault/a.md", b"hello"), ("vault/b.md", b"world")];
        let path = seal(dir.path(), "two.hqbak", |t| good(t, files, &db));
        let ok = Limits::default();
        assert!(extract(&path, &secret(), Unpack::Nothing, &ok).is_ok());
        let few = Limits {
            max_entries: 2,
            ..Limits::default()
        };
        assert!(matches!(
            extract(&path, &secret(), Unpack::Nothing, &few),
            Err(BackupError::Refused(_))
        ));
        let small = Limits {
            max_bytes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            extract(&path, &secret(), Unpack::Nothing, &small),
            Err(BackupError::Refused(_))
        ));
    }

    /// A declared size that is a lie cannot make a restore write more than the entry holds or
    /// read past it.
    #[test]
    fn an_entry_larger_than_the_limit_is_refused_before_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let big = vec![7u8; 200_000];
        let path = seal(dir.path(), "big.hqbak", |t| {
            good(t, &[("vault/big.bin", &big[..])], &db)
        });
        let into = dir.path().join("into");
        let err = restore(&RestoreOptions {
            archive: &path,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: Some(100),
        })
        .unwrap_err();
        assert!(matches!(err, BackupError::Refused(_)), "{err}");
        assert!(!into.exists() && leftovers(dir.path()).is_empty());
    }

    #[test]
    fn verify_checks_the_database_not_only_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let junk = b"this is not a SQLite database at all, only text".to_vec();
        let bad = seal(dir.path(), "bad.hqbak", |t| {
            good(t, &[("vault/a.md", b"hello")], &junk)
        });
        assert!(verify(&bad, &secret()).is_err());
        let db = db_bytes(dir.path());
        let fine = seal(dir.path(), "fine.hqbak", |t| {
            good(t, &[("vault/a.md", b"hello")], &db)
        });
        assert_eq!(verify(&fine, &secret()).unwrap().file_count(), 1);
    }

    /// A modification time past what the clock can hold must not panic a restore.
    #[test]
    fn a_huge_modification_time_is_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let path = seal(dir.path(), "mtime.hqbak", |t| {
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mtime(u64::MAX);
            t.append_data(&mut header, "vault/a.md", &b"hello"[..])
                .unwrap();
            file(t, DB_ENTRY, &db);
            manifest_entry(t, &manifest(&[("vault/a.md", b"hello")], &db));
        });
        let into = dir.path().join("restored");
        restore(&RestoreOptions {
            archive: &path,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: None,
        })
        .unwrap();
        assert_eq!(fs::read(into.join("a.md")).unwrap(), b"hello");
    }

    #[test]
    fn tar_records_hq_never_writes_are_refused_before_tar_buffers_them() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let cases: [(&str, tar::EntryType, u64); 4] = [
            ("pax", tar::EntryType::XHeader, 10),
            ("global pax", tar::EntryType::XGlobalHeader, 10),
            ("sparse", tar::EntryType::GNUSparse, 10),
            (
                "a long name that claims a gigabyte",
                tar::EntryType::GNULongName,
                1 << 30,
            ),
        ];
        for (label, kind, claim) in cases {
            let path = seal(dir.path(), "bad.hqbak", |t| {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(kind);
                header.set_size(claim);
                header.set_mode(0o644);
                header.as_old_mut().name[..4].copy_from_slice(b"junk");
                header.set_cksum();
                let data = if claim <= 10 {
                    &b"0123456789"[..claim as usize]
                } else {
                    &[][..]
                };
                t.append(&header, data).unwrap();
                good(t, &[], &db);
            });
            let err = refuse(dir.path(), &path);
            assert!(
                matches!(&err, BackupError::Refused(m) if m.contains("HQ never writes")),
                "{label}: {err}"
            );
        }
    }

    /// A size field that is nearly the largest number must be refused, not wrap or panic.
    #[test]
    fn a_size_at_the_top_of_the_range_is_refused_not_overflowed() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let path = seal(dir.path(), "huge.hqbak", |t| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(u64::MAX - 100);
            header.set_mode(0o644);
            header.as_old_mut().name[..8].copy_from_slice(b"vault/a\0");
            header.set_cksum();
            t.append(&header, &[][..]).unwrap();
            good(t, &[], &db);
        });
        let err = refuse(dir.path(), &path);
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn a_long_but_real_path_still_restores_through_the_long_name_record() {
        let dir = tempfile::tempdir().unwrap();
        let db = db_bytes(dir.path());
        let long = format!("vault/{}/{}/note.md", "x".repeat(120), "y".repeat(120));
        let path = seal(dir.path(), "long.hqbak", |t| {
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            t.append_data(&mut header, &long, &b"hello"[..]).unwrap();
            file(t, DB_ENTRY, &db);
            manifest_entry(t, &manifest(&[(long.as_str(), b"hello")], &db));
        });
        let into = dir.path().join("restored");
        restore(&RestoreOptions {
            archive: &path,
            into: &into,
            secret: &secret(),
            config_out: None,
            reindex: false,
            max_bytes: None,
        })
        .unwrap();
        let rel = long.trim_start_matches("vault/");
        assert_eq!(fs::read(into.join(rel)).unwrap(), b"hello");
    }
}

/// Reading files other programs may be writing, and the private work folders.
#[cfg(test)]
mod staging {
    use super::*;

    fn age(path: &Path, by: Duration) {
        File::open(path)
            .unwrap()
            .set_modified(SystemTime::now() - by)
            .unwrap();
    }

    #[test]
    fn each_kind_of_file_is_taken_waited_for_or_left_out_for_a_stated_reason() {
        let dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name);

        let staged = |name: &str| stage_once(&at(name), work.path(), 0).unwrap();
        assert!(matches!(staged("missing"), Staging::Left(m) if m.contains("removed")));

        fs::create_dir(at("folder")).unwrap();
        assert!(matches!(staged("folder"), Staging::Left(m) if m.contains("plain file")));

        fs::write(at("fresh"), "just written").unwrap();
        assert!(matches!(staged("fresh"), Staging::Later));

        fs::write(at("quiet"), "settled").unwrap();
        age(&at("quiet"), Duration::from_secs(60));
        match staged("quiet") {
            Staging::Ready(Staged::Memory(bytes), size, _) => {
                assert_eq!((bytes.as_slice(), size), (&b"settled"[..], 7));
            }
            _ => panic!("a settled file must be taken whole"),
        }

        let big = vec![3u8; IN_MEMORY_LIMIT as usize + 1];
        fs::write(at("big"), &big).unwrap();
        age(&at("big"), Duration::from_secs(60));
        match staged("big") {
            Staging::Ready(Staged::Disk(copy), size, _) => {
                assert_eq!(size, big.len() as u64);
                assert_eq!(fs::read(copy).unwrap(), big);
            }
            _ => panic!("a big file must be staged on disk"),
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(at("quiet"), at("link")).unwrap();
            assert!(matches!(staged("link"), Staging::Left(m) if m.contains("link")));
        }
    }

    #[test]
    fn a_work_folder_is_private_unique_and_removed_when_done() {
        let (first, second) = (
            make_work_dir(None, 0).unwrap(),
            make_work_dir(None, 0).unwrap(),
        );
        assert_ne!(first.path(), second.path());
        let name = first.path().file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(WORK_PREFIX), "{name}");
        let kept = first.path().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&kept).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        fs::write(kept.join("plaintext"), "x").unwrap();
        drop(first);
        assert!(!kept.exists());
        assert!(second.path().is_dir());
    }

    #[test]
    fn only_old_work_folders_named_like_ours_are_swept() {
        let parent = tempfile::tempdir().unwrap();
        let stale_name = format!("{WORK_PREFIX}{}", "ab".repeat(WORK_NONCE_BYTES));
        let make = |name: &str| {
            let path = parent.path().join(name);
            fs::create_dir(&path).unwrap();
            path
        };
        let old = make(&stale_name);
        let fresh = make(&format!("{WORK_PREFIX}{}", "cd".repeat(WORK_NONCE_BYTES)));
        let other = make(&format!("{WORK_PREFIX}short"));
        let unrelated = make("somebody-elses-folder");
        for path in [&old, &other, &unrelated] {
            age(path, 3 * STALE_WORK);
        }
        sweep_stale_work(parent.path());
        assert!(!old.exists());
        assert!(fresh.exists() && other.exists() && unrelated.exists());
    }

    #[test]
    fn a_backup_never_replaces_a_file_that_appeared_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, out) = (
            dir.path().join("b.hqbak.partial"),
            dir.path().join("b.hqbak"),
        );
        fs::write(&partial, "finished").unwrap();
        fs::write(&out, "somebody else's").unwrap();
        let err = publish(&partial, &out).unwrap_err();
        assert!(matches!(err, BackupError::Refused(_)), "{err}");
        assert_eq!(fs::read_to_string(&out).unwrap(), "somebody else's");
        fs::remove_file(&out).unwrap();
        publish(&partial, &out).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "finished");
        assert!(!partial.exists());
    }

    #[test]
    fn a_work_folder_that_would_not_fit_is_not_used() {
        let err = make_work_dir(None, u64::MAX).unwrap_err().to_string();
        assert!(err.contains("TMPDIR"), "{err}");
        #[cfg(unix)]
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|free| free > 0));
        assert!(make_work_dir(None, 1024).is_ok());
    }

    #[test]
    fn a_stale_folder_beside_the_backup_is_swept_even_when_the_temporary_folder_is_used() {
        let beside = tempfile::tempdir().unwrap();
        let stale = beside
            .path()
            .join(format!("{WORK_PREFIX}{}", "ab".repeat(WORK_NONCE_BYTES)));
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("vault.db"), "plaintext").unwrap();
        File::open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - 3 * STALE_WORK)
            .unwrap();
        let work = make_work_dir(Some(beside.path()), 0).unwrap();
        assert!(
            !work.path().starts_with(beside.path()),
            "the temporary folder works"
        );
        assert!(!stale.exists(), "the leftover next to the backups is gone");
    }
}
