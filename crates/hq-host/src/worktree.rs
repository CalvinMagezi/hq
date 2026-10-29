//! A git worktree per agent, so two agents on one repository never share a working tree
//! (docs/plans/remote-driven-teams.md, phase 4). A writer gets its own branch, a reviewer a
//! detached checkout of the commit it reviews. Worktrees live under the host's HQ folder,
//! `worktrees/<repository>/<session>`, and are made from a folder HQ already approved, so the
//! agent starts somewhere the owner has already said yes to.

use crate::names::check_folder_name;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The folder under the HQ folder that holds every worktree.
pub const WORKTREES_DIR: &str = "worktrees";
/// Longest branch or ref name taken.
const MAX_REF: usize = 200;

/// What a worktree is checked out at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// A branch for a writer: checked out if it exists, else made from the folder's current
    /// commit. Refused while another worktree has it checked out.
    Branch(String),
    /// A commit for a reviewer, checked out detached.
    Detached(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    /// The repository's main folder.
    pub repo: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    /// False when it already existed (a retry or a resume).
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Head {
    pub sha: String,
    pub subject: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Removed {
    pub removed: bool,
    /// It has changes nobody committed, so it was kept.
    pub dirty: bool,
}

#[derive(Debug)]
pub enum WorktreeError {
    Invalid(String),
    NotARepo(String),
    BranchInUse(String),
    Git(String),
}

impl WorktreeError {
    pub fn code(&self) -> &'static str {
        match self {
            WorktreeError::Invalid(_) => "invalid",
            WorktreeError::NotARepo(_) => "not_a_repo",
            WorktreeError::BranchInUse(_) => "branch_in_use",
            WorktreeError::Git(_) => "git_failed",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            WorktreeError::Invalid(m)
            | WorktreeError::NotARepo(m)
            | WorktreeError::BranchInUse(m)
            | WorktreeError::Git(m) => m,
        }
    }
}

impl From<std::io::Error> for WorktreeError {
    fn from(e: std::io::Error) -> Self {
        WorktreeError::Invalid(e.to_string())
    }
}

/// Runs git in `dir` with no prompt and no inherited repository from the environment.
fn git(dir: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|e| WorktreeError::Git(format!("git could not run: {e}")))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    // git says what it was doing before it says what went wrong: the `fatal:` or `error:` line
    // is the one that matters.
    let err = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = err
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let line = lines
        .iter()
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(lines.last())
        .copied()
        .unwrap_or("git failed");
    Err(WorktreeError::Git(line.to_string()))
}

fn path_arg(p: &Path) -> Result<&str, WorktreeError> {
    p.to_str()
        .ok_or_else(|| WorktreeError::Invalid("that folder's name is not plain text".into()))
}

/// A name git takes as a branch, and that no command line could read as an option.
fn check_branch(dir: &Path, name: &str) -> Result<(), WorktreeError> {
    if name.is_empty()
        || name.starts_with('-')
        || name.len() > MAX_REF
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(WorktreeError::Invalid(format!(
            "'{name}' is not a branch name"
        )));
    }
    git(dir, &["check-ref-format", "--branch", name])
        .map(|_| ())
        .map_err(|_| WorktreeError::Invalid(format!("'{name}' is not a branch name")))
}

/// The commit `r` names in `dir`.
fn commit_of(dir: &Path, r: &str) -> Result<String, WorktreeError> {
    if r.is_empty()
        || r.starts_with('-')
        || r.len() > MAX_REF
        || r.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(WorktreeError::Invalid(format!("'{r}' is not a commit")));
    }
    git(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{r}^{{commit}}"),
        ],
    )
    .map_err(|_| WorktreeError::Invalid(format!("the repository has no commit '{r}'")))
}

/// The repository's main folder: the parent of the shared `.git`, whichever worktree `dir` is in.
fn main_folder(dir: &Path) -> Result<PathBuf, WorktreeError> {
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map_err(|_| {
        WorktreeError::NotARepo(format!("{} is not in a git repository", dir.display()))
    })?;
    let common = PathBuf::from(common);
    let main = if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().map(Path::to_path_buf).unwrap_or(common)
    } else {
        // A bare repository: its own folder names it.
        common
    };
    Ok(main.canonicalize()?)
}

fn current_branch(dir: &Path) -> Option<String> {
    git(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty())
}

/// The commit a folder is at, its subject and branch. `None` outside a repository or before
/// its first commit.
pub fn head(dir: &Path) -> Option<Head> {
    let line = git(dir, &["log", "-1", "--format=%H%x1f%s"]).ok()?;
    let (sha, subject) = line.split_once('\u{1f}')?;
    Some(Head {
        sha: sha.to_string(),
        subject: subject.chars().take(200).collect(),
        branch: current_branch(dir),
    })
}

/// A folder name from the repository's own name: letters, digits, `.`, `_` and `-` kept.
fn folder_for(main: &Path) -> String {
    let raw = main
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let clean: String = raw
        .trim_end_matches(".git")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches(|c| c == '.' || c == '-').to_string();
    if clean.is_empty() {
        "repo".into()
    } else {
        clean
    }
}

/// Makes (or finds again) the worktree for session `session` of the repository `repo` is in,
/// under `root/worktrees/<repository>/<session>`.
pub fn add(
    root: &Path,
    repo: &Path,
    session: &str,
    base: &Base,
) -> Result<Worktree, WorktreeError> {
    let session = check_folder_name(session).map_err(|e| WorktreeError::Invalid(e.to_string()))?;
    let repo = repo
        .canonicalize()
        .map_err(|_| WorktreeError::NotARepo(format!("{} does not exist", repo.display())))?;
    let main = main_folder(&repo)?;
    let dir = root.join(WORKTREES_DIR).join(folder_for(&main));
    fs::create_dir_all(&dir)?;
    let target = dir.join(session);
    if target.exists() {
        return reuse(&main, &target, base);
    }
    let target_arg = path_arg(&target)?;
    let added = match base {
        Base::Branch(name) => {
            check_branch(&repo, name)?;
            let exists = git(
                &repo,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{name}"),
                ],
            )
            .is_ok();
            if exists {
                git(&repo, &["worktree", "add", target_arg, name])
            } else {
                git(&repo, &["worktree", "add", "-b", name, target_arg, "HEAD"])
            }
        }
        Base::Detached(r) => {
            let sha = commit_of(&repo, r)?;
            git(&repo, &["worktree", "add", "--detach", target_arg, &sha])
        }
    };
    if let Err(WorktreeError::Git(msg)) = &added
        && (msg.contains("already checked out") || msg.contains("already used by worktree"))
    {
        return Err(WorktreeError::BranchInUse(msg.clone()));
    }
    added?;
    let path = target.canonicalize()?;
    let head = head(&path).map(|h| h.sha).unwrap_or_default();
    Ok(Worktree {
        branch: current_branch(&path),
        path,
        repo: main,
        head,
        created: true,
    })
}

/// A worktree that already exists at `target` is taken again when it belongs to the same
/// repository and is on what was asked for, so a retried launch or a resume finds it.
fn reuse(main: &Path, target: &Path, base: &Base) -> Result<Worktree, WorktreeError> {
    let path = target.canonicalize()?;
    let same_repo = main_folder(&path).is_ok_and(|m| m == main);
    let top = git(&path, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from);
    if !same_repo || top.and_then(|t| t.canonicalize().ok()).as_deref() != Some(path.as_path()) {
        return Err(WorktreeError::Invalid(format!(
            "{} already exists and is not a worktree of this repository",
            path.display()
        )));
    }
    let branch = current_branch(&path);
    let fits = match base {
        Base::Branch(name) => branch.as_deref() == Some(name.as_str()),
        Base::Detached(_) => branch.is_none(),
    };
    if !fits {
        return Err(WorktreeError::Invalid(format!(
            "{} already exists on {}",
            path.display(),
            branch
                .as_deref()
                .map_or("a detached commit".to_string(), |b| format!("branch {b}"))
        )));
    }
    let head = head(&path).map(|h| h.sha).unwrap_or_default();
    Ok(Worktree {
        path,
        repo: main.to_path_buf(),
        branch,
        head,
        created: false,
    })
}

/// Removes a worktree HQ made, unless it has changes nobody committed: those are kept, and the
/// caller says where. The branch stays, with every commit on it. A path that is already gone
/// counts as removed.
pub fn remove(root: &Path, path: &Path) -> Result<Removed, WorktreeError> {
    let holder = root.join(WORKTREES_DIR);
    let Ok(holder) = holder.canonicalize() else {
        return Ok(Removed {
            removed: true,
            dirty: false,
        });
    };
    let Ok(path) = path.canonicalize() else {
        return Ok(Removed {
            removed: true,
            dirty: false,
        });
    };
    // Only `worktrees/<repository>/<session>`, never the holder or a repository's folder.
    let inside = path
        .strip_prefix(&holder)
        .is_ok_and(|rest| rest.components().count() == 2);
    if !inside {
        return Err(WorktreeError::Invalid(format!(
            "{} is not a worktree HQ made",
            path.display()
        )));
    }
    let status = git(&path, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Ok(Removed {
            removed: false,
            dirty: true,
        });
    }
    git(&path, &["worktree", "remove", path_arg(&path)?])?;
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(Removed {
        removed: true,
        dirty: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository with one commit, and the HQ folder worktrees go under.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("my app");
        fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["commit", "-q", "--allow-empty", "-m", "first"]);
        let root = tmp.path().join("HQ");
        fs::create_dir_all(&root).unwrap();
        (tmp, repo, root)
    }

    fn commit(dir: &Path, message: &str) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-q", "--allow-empty", "-m", message])
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }

    #[test]
    fn a_writer_gets_its_own_branch_and_only_one_writer_has_it() {
        let (_t, repo, root) = setup();
        let wt = add(&root, &repo, "hs-1", &Base::Branch("hq/fr-12".into())).unwrap();
        assert!(wt.created);
        assert_eq!(wt.branch.as_deref(), Some("hq/fr-12"));
        assert!(
            wt.path.ends_with("worktrees/my-app/hs-1"),
            "{}",
            wt.path.display()
        );
        assert_eq!(wt.repo, repo.canonicalize().unwrap());

        let again = add(&root, &repo, "hs-1", &Base::Branch("hq/fr-12".into())).unwrap();
        assert!(!again.created, "a retry finds the same worktree");
        assert_eq!(again.path, wt.path);

        let taken = add(&root, &repo, "hs-2", &Base::Branch("hq/fr-12".into())).unwrap_err();
        assert_eq!(taken.code(), "branch_in_use", "{}", taken.message());
        let other = add(&root, &repo, "hs-1", &Base::Branch("other".into())).unwrap_err();
        assert_eq!(other.code(), "invalid");
    }

    #[test]
    fn a_reviewer_gets_the_writers_commit_detached() {
        let (_t, repo, root) = setup();
        let writer = add(&root, &repo, "hs-w", &Base::Branch("feature".into())).unwrap();
        commit(&writer.path, "add the parser");
        let h = head(&writer.path).unwrap();
        assert_eq!(h.subject, "add the parser");
        assert_eq!(h.branch.as_deref(), Some("feature"));

        let reviewer = add(&root, &repo, "hs-r", &Base::Detached(h.sha.clone())).unwrap();
        assert_eq!(reviewer.branch, None);
        assert_eq!(reviewer.head, h.sha);
        // From inside the writer's worktree too: the same repository.
        let from_worktree = add(
            &root,
            &writer.path,
            "hs-r2",
            &Base::Detached("feature".into()),
        )
        .unwrap();
        assert_eq!(from_worktree.repo, repo.canonicalize().unwrap());
    }

    #[test]
    fn names_that_could_be_options_or_are_not_refs_are_refused() {
        let (_t, repo, root) = setup();
        for bad in ["-f", "--force", "a b", "a..b", ""] {
            let err = add(&root, &repo, "hs-1", &Base::Branch(bad.into())).unwrap_err();
            assert_eq!(err.code(), "invalid", "{bad}");
        }
        let err = add(&root, &repo, "hs-1", &Base::Detached("--all".into())).unwrap_err();
        assert_eq!(err.code(), "invalid");
        let err = add(&root, &repo, "hs-1", &Base::Detached("nosuchref".into())).unwrap_err();
        assert_eq!(err.code(), "invalid");
        let err = add(&root, &repo, "../x", &Base::Branch("b".into())).unwrap_err();
        assert_eq!(err.code(), "invalid");
        let plain = tempfile::tempdir().unwrap();
        let err = add(&root, plain.path(), "hs-1", &Base::Branch("b".into())).unwrap_err();
        assert_eq!(err.code(), "not_a_repo");
    }

    #[test]
    fn a_clean_worktree_is_removed_and_a_dirty_one_kept() {
        let (_t, repo, root) = setup();
        let wt = add(&root, &repo, "hs-1", &Base::Branch("work".into())).unwrap();
        commit(&wt.path, "done");
        fs::write(wt.path.join("notes.txt"), "not committed").unwrap();
        let kept = remove(&root, &wt.path).unwrap();
        assert!(!kept.removed && kept.dirty);
        assert!(wt.path.exists());

        fs::remove_file(wt.path.join("notes.txt")).unwrap();
        let gone = remove(&root, &wt.path).unwrap();
        assert!(gone.removed && !gone.dirty);
        assert!(!wt.path.exists());
        assert!(
            git(&repo, &["show-ref", "--verify", "refs/heads/work"]).is_ok(),
            "the branch and its commits stay"
        );
        assert!(
            remove(&root, &wt.path).unwrap().removed,
            "already gone counts as removed"
        );
        // Resuming makes it again on the same branch.
        let back = add(&root, &repo, "hs-1", &Base::Branch("work".into())).unwrap();
        assert_eq!(head(&back.path).unwrap().subject, "done");
    }

    #[test]
    fn only_worktrees_hq_made_are_removed() {
        let (_t, repo, root) = setup();
        add(&root, &repo, "hs-1", &Base::Branch("work".into())).unwrap();
        for not_ours in [
            repo.clone(),
            root.join(WORKTREES_DIR),
            root.join(WORKTREES_DIR).join("my-app"),
        ] {
            let err = remove(&root, &not_ours).unwrap_err();
            assert_eq!(err.code(), "invalid", "{}", not_ours.display());
        }
    }
}
