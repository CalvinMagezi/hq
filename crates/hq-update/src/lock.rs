use crate::error::{Result, UpdateError};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;

/// Exclusive, non-blocking `flock` held for the life of the value. The
/// kernel drops it if the process dies, so a crash never leaves a stale lock.
#[derive(Debug)]
pub struct UpdateLock {
    _file: File,
}

impl UpdateLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        // SAFETY: the fd is valid for the duration of the call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            return if err.kind() == std::io::ErrorKind::WouldBlock {
                Err(UpdateError::Locked)
            } else {
                Err(err.into())
            };
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_holder_is_refused_until_the_first_drops() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("sub/update.lock");
        let first = UpdateLock::acquire(&path).unwrap();
        assert!(matches!(
            UpdateLock::acquire(&path),
            Err(UpdateError::Locked)
        ));
        drop(first);
        UpdateLock::acquire(&path).unwrap();
    }
}
