//! Checking a program the person picked as their `hq`, before the app will ever run it.
//!
//! The picker itself (an open panel) lives in the shell; everything that decides whether the
//! pick is acceptable lives here so it can be tested without a window.

use crate::host::{VERSION_TIMEOUT, hq_version_within};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Why a pick was not taken. The page shows a sentence for each; none of them says what is inside the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The path or one of its folders can be written by anyone on this computer.
    Refused,
    /// The file ran, but did not answer like HQ.
    NotHq,
    /// There is nothing at that path.
    NotFound,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Refused => "refused",
            Self::NotHq => "not_hq",
            Self::NotFound => "not_found",
        })
    }
}

/// A program that passed every check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Picked {
    pub path: PathBuf,
    pub version: String,
}

/// Whether anyone but the owner could swap this path for another program: the path or any
/// folder above it (as written, and as it resolves) is writable by everyone.
fn world_writable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    // A sticky shared folder such as /tmp still lets anyone add a file, so it counts.
    let open = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o002 != 0);
    let walk = |start: &Path| start.ancestors().any(open);
    walk(path) || std::fs::canonicalize(path).is_ok_and(|real| walk(&real))
}

/// Checks the pick: it must sit where only its owner can change it, and answer `hq --version`.
pub fn check_program(path: &Path) -> Result<Picked, Refusal> {
    check_program_within(path, VERSION_TIMEOUT)
}

/// `check_program` with its own time limit, so tests need not wait.
pub fn check_program_within(path: &Path, limit: Duration) -> Result<Picked, Refusal> {
    if !path.is_absolute() {
        return Err(Refusal::NotFound);
    }
    // The location is judged before the file is looked at, so a missing /tmp/hq is refused too.
    if world_writable(path) {
        return Err(Refusal::Refused);
    }
    if !path.exists() {
        return Err(Refusal::NotFound);
    }
    match hq_version_within(path, limit) {
        Some(version) => Ok(Picked {
            path: path.to_path_buf(),
            version,
        }),
        None => Err(Refusal::NotHq),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn a_program_in_a_shared_folder_is_refused_even_when_it_is_not_there() {
        assert_eq!(check_program(Path::new("/tmp/hq")), Err(Refusal::Refused));
        let shared = tempfile::tempdir().unwrap();
        std::fs::set_permissions(shared.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let hq = script(shared.path(), "hq", "echo hq 1.0.0");
        assert_eq!(check_program(&hq), Err(Refusal::Refused));
    }

    #[test]
    fn a_program_in_a_private_folder_that_answers_like_hq_is_taken() {
        let dir = private_dir();
        let hq = script(dir.path(), "hq", "echo 'hq 1.2.3'");
        let picked = check_program(&hq).unwrap();
        assert_eq!(picked.version, "hq 1.2.3");
        assert_eq!(picked.path, hq);
    }

    #[test]
    fn a_program_that_is_not_hq_or_not_there_is_told_apart() {
        let dir = private_dir();
        let other = script(dir.path(), "other", "echo nope");
        assert_eq!(check_program(&other), Err(Refusal::NotHq));
        assert_eq!(
            check_program(&dir.path().join("missing")),
            Err(Refusal::NotFound)
        );
        assert_eq!(check_program(Path::new("hq")), Err(Refusal::NotFound));
    }

    #[test]
    fn a_program_that_hangs_is_not_hq() {
        let dir = private_dir();
        let slow = script(dir.path(), "hq", "sleep 30");
        assert_eq!(
            check_program_within(&slow, Duration::from_millis(300)),
            Err(Refusal::NotHq)
        );
    }
}
