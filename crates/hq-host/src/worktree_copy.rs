//! Gives a new worktree the files git does not track but the work needs, such as `.env` and
//! `node_modules`, so an agent does not start with a reinstall or a cold build. The repository
//! opts in with a `.worktreeinclude` file at its root (gitignore syntax, the same file Claude
//! Code reads). Only paths that file names AND git ignores are copied, so nothing git would
//! show as a change is ever planted. Copies are copy-on-write clones where the file system
//! has them (APFS, btrfs, XFS), so a large `node_modules` costs almost no time or disk.

use std::io::Write;
use std::path::{Component, Path};
use std::process::{Command, Stdio};

/// The file that names what a new worktree gets.
pub const INCLUDE_FILE: &str = ".worktreeinclude";
/// Most paths copied into one worktree; a pattern that matches more is too broad to be meant.
const MAX_PATHS: usize = 500;

/// What setting a worktree up did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Copied {
    pub copied: Vec<String>,
    /// Each with the reason it was not copied.
    pub not_copied: Vec<String>,
}

/// Runs git in `dir` with `input` on stdin; `None` when git cannot run. Exit status is the
/// caller's to read, since `check-ignore` exits 1 for "none of these".
fn git_lines(dir: &Path, args: &[&str], input: Option<&str>) -> Option<(bool, Vec<String>)> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect();
    Some((out.status.success(), lines))
}

/// A path relative to the repository that stays inside it.
fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

/// The untracked paths `.worktreeinclude` names that git also ignores, relative to `main`.
/// A directory comes back once, as itself, not file by file.
fn wanted(main: &Path, include: &Path) -> Result<Vec<String>, String> {
    let include_arg = format!("--exclude-from={}", include.display());
    let (ok, named) = git_lines(
        main,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--directory",
            &include_arg,
        ],
        None,
    )
    .ok_or("git could not run")?;
    if !ok {
        return Err("git could not list the files .worktreeinclude names".into());
    }
    let named: Vec<String> = named
        .into_iter()
        .map(|p| p.trim_end_matches('/').to_string())
        .collect();
    if named.is_empty() {
        return Ok(Vec::new());
    }
    let input = named.join("\n") + "\n";
    // Exit 1 means none of them is ignored: nothing to copy, not a failure.
    let (_, ignored) =
        git_lines(main, &["check-ignore", "--stdin"], Some(&input)).ok_or("git could not run")?;
    Ok(ignored
        .into_iter()
        .map(|p| p.trim_end_matches('/').to_string())
        .collect())
}

/// Makes `dst`'s parent folder and checks it resolves inside `target`, so a tracked symlink in
/// the path cannot send a copy outside the worktree.
fn parent_inside(dst: &Path, target: &Path) -> Result<(), String> {
    let parent = dst.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let real = parent.canonicalize().map_err(|e| e.to_string())?;
    let root = target.canonicalize().map_err(|e| e.to_string())?;
    if real.starts_with(&root) {
        return Ok(());
    }
    Err("its folder is a link out of the worktree".into())
}

/// Copies `src` to `dst` (which does not exist yet), as a clone where the file system can.
fn copy_path(src: &Path, dst: &Path) -> Result<(), String> {
    let run = |flags: &[&str]| {
        Command::new("cp")
            .args(flags)
            .arg(src)
            .arg(dst)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("cp could not run: {e}"))
    };
    // `-c` asks macOS for clonefile(2); on a volume without clones it fails and a plain copy
    // runs. GNU cp clones with `--reflink=auto` and copies where it cannot.
    let tries: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["-c", "-R", "-p"], &["-R", "-p"]]
    } else if cfg!(target_os = "linux") {
        &[&["-R", "-p", "--reflink=auto"], &["-R", "-p"]]
    } else {
        &[&["-R", "-p"]]
    };
    let mut last = String::new();
    for flags in tries {
        let out = run(flags)?;
        if out.status.success() {
            return Ok(());
        }
        last = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // A failed clone can leave part of a tree behind; the next try starts clean.
        let _ = std::fs::remove_dir_all(dst).or_else(|_| std::fs::remove_file(dst));
    }
    Err(if last.is_empty() {
        "cp failed".into()
    } else {
        last
    })
}

/// Copies into the new worktree `target` what `.worktreeinclude` names from the repository's
/// main folder `main`. The worktree's own `.worktreeinclude` wins, so the list matches the
/// commit it was made at; the main folder's is used when the commit has none. Never fails the
/// worktree: what could not be copied is reported.
pub fn copy_included(main: &Path, target: &Path) -> Copied {
    let mut done = Copied::default();
    let include = [target.join(INCLUDE_FILE), main.join(INCLUDE_FILE)]
        .into_iter()
        .find(|p| p.is_file());
    let Some(include) = include else {
        return done;
    };
    let paths = match wanted(main, &include) {
        Ok(p) => p,
        Err(e) => {
            done.not_copied.push(format!("{INCLUDE_FILE}: {e}"));
            return done;
        }
    };
    if paths.len() > MAX_PATHS {
        done.not_copied.push(format!(
            "{INCLUDE_FILE}: names {} paths, more than {MAX_PATHS}; narrow it to directories",
            paths.len()
        ));
        return done;
    }
    for rel in paths {
        if !safe_relative(&rel) {
            done.not_copied
                .push(format!("{rel}: not a path inside the repository"));
            continue;
        }
        let dst = target.join(&rel);
        if dst.symlink_metadata().is_ok() {
            done.not_copied
                .push(format!("{rel}: already in the worktree"));
            continue;
        }
        if let Err(e) = parent_inside(&dst, target) {
            done.not_copied.push(format!("{rel}: {e}"));
            continue;
        }
        match copy_path(&main.join(&rel), &dst) {
            Ok(()) => done.copied.push(rel),
            Err(e) => done.not_copied.push(format!("{rel}: {e}")),
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A repository that ignores `.env`, `node_modules/` and `*.log`, with each present.
    fn repo(include: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("app");
        fs::create_dir_all(main.join("node_modules/left-pad")).unwrap();
        fs::create_dir_all(main.join("web/node_modules/x")).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join(".gitignore"), ".env\nnode_modules/\n*.log\n").unwrap();
        if let Some(text) = include {
            fs::write(main.join(INCLUDE_FILE), text).unwrap();
        }
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-q", "-m", "first"]);
        fs::write(main.join(".env"), "KEY=1").unwrap();
        fs::write(main.join("node_modules/left-pad/index.js"), "x").unwrap();
        fs::write(main.join("web/node_modules/x/i.js"), "y").unwrap();
        fs::write(main.join("debug.log"), "noise").unwrap();
        // Untracked but not ignored: git would show it, so it is never planted.
        fs::write(main.join("draft.env"), "z").unwrap();
        (tmp, main)
    }

    #[test]
    fn copies_only_what_the_include_file_names_and_git_ignores() {
        let (tmp, main) = repo(Some(".env\nnode_modules/\ndraft.env\n"));
        let target = tmp.path().join("wt");
        git(&main, &["worktree", "add", "-q", target.to_str().unwrap()]);
        let mut done = copy_included(&main, &target);
        done.copied.sort();
        assert_eq!(
            done.copied,
            [".env", "node_modules", "web/node_modules"],
            "{done:?}"
        );
        assert_eq!(fs::read_to_string(target.join(".env")).unwrap(), "KEY=1");
        assert!(target.join("node_modules/left-pad/index.js").is_file());
        assert!(target.join("web/node_modules/x/i.js").is_file());
        assert!(!target.join("debug.log").exists(), "not named");
        assert!(!target.join("draft.env").exists(), "not ignored");
        let status = Command::new("git")
            .arg("-C")
            .arg(&target)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(status.stdout.is_empty(), "the copies are invisible to git");
    }

    #[test]
    fn no_include_file_copies_nothing() {
        let (tmp, main) = repo(None);
        let target = tmp.path().join("wt");
        git(&main, &["worktree", "add", "-q", target.to_str().unwrap()]);
        assert_eq!(copy_included(&main, &target), Copied::default());
        assert!(!target.join(".env").exists());
    }

    #[test]
    fn a_path_already_in_the_worktree_is_left_alone() {
        let (tmp, main) = repo(Some(".env\n"));
        let target = tmp.path().join("wt");
        git(&main, &["worktree", "add", "-q", target.to_str().unwrap()]);
        fs::write(target.join(".env"), "MINE=1").unwrap();
        let done = copy_included(&main, &target);
        assert!(done.copied.is_empty());
        assert_eq!(done.not_copied.len(), 1, "{done:?}");
        assert_eq!(fs::read_to_string(target.join(".env")).unwrap(), "MINE=1");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_links_out_of_the_worktree_is_not_copied_into() {
        let (tmp, main) = repo(Some("web/node_modules/\n"));
        let target = tmp.path().join("wt");
        git(&main, &["worktree", "add", "-q", target.to_str().unwrap()]);
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::remove_dir_all(target.join("web")).ok();
        std::os::unix::fs::symlink(&outside, target.join("web")).unwrap();
        let done = copy_included(&main, &target);
        assert!(done.copied.is_empty(), "{done:?}");
        assert!(!outside.join("node_modules").exists());
    }

    #[test]
    fn only_plain_relative_paths_are_taken() {
        assert!(safe_relative("web/node_modules"));
        assert!(!safe_relative("../outside"));
        assert!(!safe_relative("/etc/passwd"));
        assert!(!safe_relative("a/../../b"));
        assert!(!safe_relative(""));
    }
}
