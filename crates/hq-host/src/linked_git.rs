//! The git directories of a linked worktree, which live outside it, and which of them a strict
//! sandbox opens.

use std::path::{Path, PathBuf};

/// The git directories a linked worktree keeps outside itself: its own `<common>/worktrees/<id>`
/// and the shared `<common>`. Without them a commit in the worktree fails under the strict wall.
pub(crate) fn linked_git_dirs(project: &Path) -> Option<(PathBuf, PathBuf)> {
    let pointer = std::fs::read_to_string(project.join(".git")).ok()?;
    let gitdir = Path::new(pointer.trim().strip_prefix("gitdir:")?.trim());
    let gitdir = project.join(gitdir).canonicalize().ok()?;
    let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = gitdir.join(common.trim()).canonicalize().ok()?;
    (gitdir.starts_with(&common)).then_some((gitdir, common))
}

/// What a commit in a linked worktree writes: the worktree's own git directory and the shared
/// object store, refs and reflogs. The rest of the shared directory (the main checkout's
/// `HEAD`, `index`, `config`, `hooks` and every sibling worktree) stays out of reach.
pub(crate) fn linked_writable(gitdir: &Path, common: &Path) -> Vec<PathBuf> {
    vec![
        gitdir.to_path_buf(),
        common.join("objects"),
        common.join("refs"),
        common.join("logs"),
    ]
}

/// Files inside the writable worktree git directory that decide where git reads its
/// configuration from, or what it runs. A planted `commondir` would point git, including the
/// owner's, at a configuration the agent wrote.
pub(crate) fn linked_protected(project: &Path, gitdir: &Path) -> Vec<PathBuf> {
    ["commondir", "gitdir", "config.worktree"]
        .iter()
        .map(|n| gitdir.join(n))
        .chain([project.join(".git")])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linked_worktree_gets_its_git_dirs_and_a_plain_checkout_gets_none() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let gitdir = root.join("main/.git/worktrees/wt");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        let wt = root.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        let (found_gitdir, common) = linked_git_dirs(&wt).expect("a linked worktree");
        assert_eq!(found_gitdir, gitdir);
        assert_eq!(common, root.join("main/.git"));
        let writable = linked_writable(&gitdir, &common);
        assert!(writable.contains(&common.join("objects")));
        assert!(
            !writable.contains(&common),
            "the shared directory itself stays closed"
        );
        let guarded = linked_protected(&wt, &gitdir);
        for name in ["commondir", "gitdir"] {
            assert!(guarded.contains(&gitdir.join(name)), "{name} read-only");
        }
        assert!(guarded.contains(&wt.join(".git")));
        std::fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        assert_eq!(
            linked_git_dirs(&wt).map(|l| l.0),
            Some(gitdir.clone()),
            "relative pointer"
        );
        let plain = root.join("plain");
        std::fs::create_dir_all(plain.join(".git")).unwrap();
        assert!(linked_git_dirs(&plain).is_none());
    }
}
