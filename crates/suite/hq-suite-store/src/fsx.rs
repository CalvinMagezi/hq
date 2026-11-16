//! Files written whole or not at all.
//!
//! A write goes to a temporary file in the same folder, is flushed to disk, and is renamed over
//! the target. A reader sees the old file or the new one, never half of either, and a crash
//! leaves at worst a stray `.tmp` file that [`sweep_partials`] removes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::error::{Result, StoreError};

/// The marker in the name of a file that is not finished.
pub const PARTIAL_MARK: &str = ".partial-";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn partial_name(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{name}{PARTIAL_MARK}{}-{n}", std::process::id()))
}

/// Flushes a folder's entry list to disk, so a rename in it survives power loss. A no-op where the
/// platform does not allow opening a folder.
pub fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn ensure_parent(path: &Path) -> Result<&Path> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::UnsafePath(path.display().to_string()))?;
    fs::create_dir_all(parent).map_err(|e| StoreError::io(parent, e))?;
    Ok(parent)
}

fn write_partial(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let tmp = partial_name(path);
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| StoreError::io(&tmp, e))?;
    f.write_all(bytes).map_err(|e| StoreError::io(&tmp, e))?;
    f.sync_all().map_err(|e| StoreError::io(&tmp, e))?;
    Ok(tmp)
}

/// Writes `bytes` to `path`, replacing what was there, atomically.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = ensure_parent(path)?;
    let tmp = write_partial(path, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(StoreError::io(path, e));
    }
    sync_dir(parent);
    Ok(())
}

/// Writes `bytes` to `path` only if nothing is there. `Ok(false)` means it already existed and
/// was left alone. Two writers racing for one path cannot both win.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<bool> {
    let parent = ensure_parent(path)?;
    let tmp = write_partial(path, bytes)?;
    // A hard link fails if the target exists, which makes "create if absent" one step. Some
    // file systems (an SD card's FAT) have no links; there a check and a rename is the best
    // that can be done.
    let created = match fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) if !path.exists() => fs::rename(&tmp, path).map(|()| true),
        Err(_) => Ok(false),
    };
    let _ = fs::remove_file(&tmp);
    match created {
        Ok(c) => {
            sync_dir(parent);
            Ok(c)
        }
        Err(e) => Err(StoreError::io(path, e)),
    }
}

/// Moves a file, refusing to replace one. Tries a rename; across file systems it copies, flushes,
/// checks the size and only then removes the source, so a failure never loses the original.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        return Err(StoreError::Exists(to.display().to_string()));
    }
    let parent = ensure_parent(to)?;
    match fs::rename(from, to) {
        Ok(()) => {
            sync_dir(parent);
            if let Some(src_parent) = from.parent() {
                sync_dir(src_parent);
            }
            Ok(())
        }
        Err(_) => {
            // Most likely a different file system. Copy through a partial file.
            let tmp = partial_name(to);
            let copied = fs::copy(from, &tmp).map_err(|e| StoreError::io(from, e))?;
            File::open(&tmp)
                .and_then(|f| f.sync_all())
                .map_err(|e| StoreError::io(&tmp, e))?;
            let want = fs::metadata(from)
                .map_err(|e| StoreError::io(from, e))?
                .len();
            if copied != want {
                let _ = fs::remove_file(&tmp);
                return Err(StoreError::Corrupt(from.display().to_string()));
            }
            fs::rename(&tmp, to).map_err(|e| StoreError::io(to, e))?;
            sync_dir(parent);
            fs::remove_file(from).map_err(|e| StoreError::io(from, e))
        }
    }
}

/// SHA-256 and length of a file, read in a stream so a large file costs no more memory than a
/// small one.
pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut f = File::open(path).map_err(|e| StoreError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| StoreError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

/// SHA-256 of bytes in memory.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Removes unfinished files a crash left under `dir` (recursively). Returns how many.
pub fn sweep_partials(dir: &Path) -> usize {
    let mut removed = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            removed += sweep_partials(&path);
        } else if ft.is_file()
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains(PARTIAL_MARK))
            && fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_partial() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/b/file.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(names.len(), 1, "no temporary file is left behind");
    }

    #[test]
    fn write_new_never_replaces() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.bin");
        assert!(write_new(&p, b"first").unwrap());
        assert!(!write_new(&p, b"second").unwrap());
        assert_eq!(fs::read(&p).unwrap(), b"first");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn racing_writers_produce_one_winner() {
        let d = tempfile::tempdir().unwrap();
        let p = std::sync::Arc::new(d.path().join("race.bin"));
        let wins: usize = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || write_new(&p, format!("writer {i}").as_bytes()).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        assert_eq!(wins, 1);
    }

    #[test]
    fn move_refuses_to_replace_and_keeps_the_source_on_refusal() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("sub/b"));
        fs::write(&a, b"A").unwrap();
        move_file(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"A");
        fs::write(&a, b"again").unwrap();
        assert!(matches!(move_file(&a, &b), Err(StoreError::Exists(_))));
        assert_eq!(fs::read(&a).unwrap(), b"again");
        assert_eq!(fs::read(&b).unwrap(), b"A");
    }

    #[test]
    fn hashing_streams_and_matches_the_bytes_form() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big");
        let data = vec![9u8; 200_000];
        fs::write(&p, &data).unwrap();
        let (h, n) = sha256_file(&p).unwrap();
        assert_eq!(n, 200_000);
        assert_eq!(h, sha256_bytes(&data));
        assert!(matches!(
            sha256_file(&d.path().join("none")),
            Err(StoreError::Io { .. })
        ));
    }

    #[test]
    fn sweep_removes_only_partials() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("x/y")).unwrap();
        fs::write(d.path().join("x/keep.txt"), "k").unwrap();
        fs::write(d.path().join(format!("x/y/.a{PARTIAL_MARK}1-1")), "p").unwrap();
        fs::write(d.path().join(format!(".b{PARTIAL_MARK}2-2")), "p").unwrap();
        assert_eq!(sweep_partials(d.path()), 2);
        assert!(d.path().join("x/keep.txt").exists());
        assert_eq!(sweep_partials(&d.path().join("missing")), 0);
    }
}
