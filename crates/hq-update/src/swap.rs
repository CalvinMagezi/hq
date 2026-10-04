//! Filesystem swaps. Every replacement is a rename on one filesystem (the
//! staged file lives next to its target), so a reader sees the old or the
//! new file, never a partial one.

use crate::error::Result;
use crate::state::{Layout, Slot, State};
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

fn copy_executable(from: &Path, to: &Path) -> io::Result<()> {
    let _ = fs::remove_file(to);
    let mut src = File::open(from)?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(to)?;
    io::copy(&mut src, &mut dst)?;
    dst.sync_all()
}

/// Replaces the live binary with `staged`, shifting `hq.1..hq.keep`.
/// `slot` describes the build being replaced and is recorded as `hq.1`.
pub fn install_binary(
    layout: &Layout,
    state: &mut State,
    staged: &Path,
    keep: usize,
    slot: Slot,
) -> Result<()> {
    fs::create_dir_all(layout.backups_dir())?;
    let had_current = layout.bin_path.exists();
    let _ = fs::remove_file(layout.backup(keep));
    for n in (1..keep).rev() {
        if layout.backup(n).exists() {
            fs::rename(layout.backup(n), layout.backup(n + 1))?;
        }
    }
    let rotated = |e: io::Error| -> Result<()> {
        unshift_backups(layout, keep);
        Err(e.into())
    };
    if had_current {
        let tmp = layout.backups_dir().join("hq.new");
        if let Err(e) =
            copy_executable(&layout.bin_path, &tmp).and_then(|_| fs::rename(&tmp, layout.backup(1)))
        {
            return rotated(e);
        }
    }
    let renamed = fs::rename(staged, &layout.bin_path);
    if renamed.is_ok() {
        crate::state::sync_dir(&layout.bin_path);
    }
    if let Err(e) = renamed {
        let _ = fs::remove_file(layout.backup(1));
        return rotated(e);
    }
    if had_current {
        state.history.insert(0, slot);
        state.history.truncate(keep);
    }
    Ok(())
}

/// Undoes the rotation done by a failed `install_binary`.
fn unshift_backups(layout: &Layout, keep: usize) {
    for n in 1..keep {
        if layout.backup(n + 1).exists() {
            let _ = fs::rename(layout.backup(n + 1), layout.backup(n));
        }
    }
}

/// Puts `hq.1` back as the live binary and shifts the older backups down.
pub fn restore_binary(layout: &Layout, state: &mut State, keep: usize) -> Result<()> {
    let newest = layout.backup(1);
    copy_executable(&newest, &layout.rollback_binary())?;
    fs::rename(layout.rollback_binary(), &layout.bin_path)?;
    crate::state::sync_dir(&layout.bin_path);
    let _ = fs::remove_file(&newest);
    for n in 1..keep {
        if layout.backup(n + 1).exists() {
            fs::rename(layout.backup(n + 1), layout.backup(n))?;
        }
    }
    if !state.history.is_empty() {
        state.history.remove(0);
    }
    Ok(())
}

/// Swaps the staged web tree in, keeping the old one as `dist.prev`.
/// Returns whether there was an old tree to keep.
pub fn install_web(layout: &Layout) -> Result<bool> {
    let (live, staged, prev) = (&layout.web_dist, layout.staged_web(), layout.prev_web());
    let had_live = live.exists();
    if prev.exists() {
        fs::remove_dir_all(&prev)?;
    }
    if had_live {
        fs::rename(live, &prev)?;
    }
    if let Err(e) = fs::rename(&staged, live) {
        if had_live {
            let _ = fs::rename(&prev, live);
        }
        return Err(e.into());
    }
    Ok(had_live)
}

/// Restores `dist.prev` as the live tree and discards the current one.
pub fn restore_web(layout: &Layout) -> Result<()> {
    let (live, prev, failed) = (&layout.web_dist, layout.prev_web(), layout.failed_web());
    if !prev.exists() {
        return Ok(());
    }
    if failed.exists() {
        fs::remove_dir_all(&failed)?;
    }
    if live.exists() {
        fs::rename(live, &failed)?;
    }
    fs::rename(&prev, live)?;
    let _ = fs::remove_dir_all(&failed);
    Ok(())
}

pub fn make_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Installed;

    fn layout(root: &Path) -> Layout {
        fs::create_dir_all(root.join("bin")).unwrap();
        Layout {
            bin_path: root.join("bin/hq"),
            web_dist: root.join("web/dist"),
            state_dir: root.join("state"),
            snapshots_dir: root.join("snapshots"),
        }
    }

    fn slot(v: &str) -> Slot {
        Slot {
            installed: Installed {
                version: v.into(),
                git_sha: format!("sha-{v}"),
            },
            db_snapshot: None,
            has_web: false,
        }
    }

    fn stage(layout: &Layout, content: &str) -> std::path::PathBuf {
        let p = layout.staged_binary();
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn keeps_three_previous_binaries_and_leaves_no_temp_files() {
        let d = tempfile::tempdir().unwrap();
        let l = layout(d.path());
        let mut st = State::default();
        fs::write(&l.bin_path, "v0").unwrap();
        for i in 1..=5 {
            let staged = stage(&l, &format!("v{i}"));
            install_binary(&l, &mut st, &staged, 3, slot(&format!("v{}", i - 1))).unwrap();
            assert_eq!(fs::read_to_string(&l.bin_path).unwrap(), format!("v{i}"));
            assert!(!staged.exists(), "staged file is consumed by the rename");
        }
        assert_eq!(fs::read_to_string(l.backup(1)).unwrap(), "v4");
        assert_eq!(fs::read_to_string(l.backup(2)).unwrap(), "v3");
        assert_eq!(fs::read_to_string(l.backup(3)).unwrap(), "v2");
        assert!(!l.backup(4).exists());
        assert_eq!(st.history.len(), 3);
        assert_eq!(st.history[0].installed.version, "v4");
        let leftovers: Vec<_> = fs::read_dir(l.backups_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 3, "{leftovers:?}");
    }

    #[test]
    fn restore_pops_newest_backup() {
        let d = tempfile::tempdir().unwrap();
        let l = layout(d.path());
        let mut st = State::default();
        fs::write(&l.bin_path, "v0").unwrap();
        for i in 1..=3 {
            let staged = stage(&l, &format!("v{i}"));
            install_binary(&l, &mut st, &staged, 3, slot(&format!("v{}", i - 1))).unwrap();
        }
        restore_binary(&l, &mut st, 3).unwrap();
        assert_eq!(fs::read_to_string(&l.bin_path).unwrap(), "v2");
        assert_eq!(fs::read_to_string(l.backup(1)).unwrap(), "v1");
        assert_eq!(fs::read_to_string(l.backup(2)).unwrap(), "v0");
        assert!(!l.backup(3).exists());
        assert_eq!(st.history.len(), 2);
        assert!(!l.rollback_binary().exists());
    }

    #[test]
    fn failed_final_rename_leaves_live_binary_and_backups_intact() {
        let d = tempfile::tempdir().unwrap();
        let l = layout(d.path());
        let mut st = State::default();
        fs::write(&l.bin_path, "v0").unwrap();
        let staged = stage(&l, "v1");
        install_binary(&l, &mut st, &staged, 3, slot("v0")).unwrap();
        let missing = d.path().join("bin/does-not-exist");
        assert!(install_binary(&l, &mut st, &missing, 3, slot("v1")).is_err());
        assert_eq!(fs::read_to_string(&l.bin_path).unwrap(), "v1");
        assert_eq!(fs::read_to_string(l.backup(1)).unwrap(), "v0");
        assert_eq!(st.history.len(), 1);
    }

    #[test]
    fn web_swap_and_restore() {
        let d = tempfile::tempdir().unwrap();
        let l = layout(d.path());
        fs::create_dir_all(&l.web_dist).unwrap();
        fs::write(l.web_dist.join("index.html"), "old").unwrap();
        fs::create_dir_all(l.staged_web()).unwrap();
        fs::write(l.staged_web().join("index.html"), "new").unwrap();
        assert!(install_web(&l).unwrap());
        assert_eq!(
            fs::read_to_string(l.web_dist.join("index.html")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(l.prev_web().join("index.html")).unwrap(),
            "old"
        );
        restore_web(&l).unwrap();
        assert_eq!(
            fs::read_to_string(l.web_dist.join("index.html")).unwrap(),
            "old"
        );
        assert!(!l.prev_web().exists() && !l.failed_web().exists());
    }
}
