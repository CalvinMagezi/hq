//! Git as the host runs it, in repositories an agent can write.
//!
//! The host runs git outside any agent's sandbox (worktree add, status and remove, the head of
//! a pane's folder, `.worktreeinclude` copies), in repositories whose `.git/config`, hooks and
//! `.gitattributes` the agent may have written under the open profile. Several of those name a
//! command git runs: `core.fsmonitor`, a hook (`post-checkout` fires on `worktree add`), a clean
//! or smudge filter, `gpg.program` behind `log.showSignature`, a transport or `core.sshCommand`
//! on a lazy fetch. Run as the agent wrote them, they would run with the host's own reach, past
//! the walls the sandbox puts around HQ. So every host git command pins those off.
//!
//! Most pins are fixed `-c` settings, which beat any repository or global value. Filter
//! drivers are named per repository, so their names are read from the configuration first and
//! each one is emptied for the command that follows. That leaves a window: an agent that adds a
//! new filter driver between the two reads gets it run. The host never asks for a diff, so diff
//! drivers (`textconv`, external diff) never fire. Closing the filter window for good means
//! running host git inside the agent's own sandbox, which is not done yet
//! (docs/AGENT_HOST.md, "Git the host runs").

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Settings that stop git running anything a repository names, whatever the repository says.
const PINS: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    // No hook directory: `/dev/null/<hook>` is never a file.
    ("core.hooksPath", "/dev/null"),
    // No transport at all, so no lazy fetch of a partial clone, no `core.sshCommand`, no
    // remote helper and no `ext::` command.
    ("protocol.allow", "never"),
    ("log.showSignature", "false"),
];

/// The keys of every filter driver git could run, per driver, and what each is set to.
const FILTER_KEYS: &[(&str, &str)] = &[
    ("clean", ""),
    ("smudge", ""),
    ("process", ""),
    // An emptied driver that was `required` would fail the command; it is no longer needed.
    ("required", "false"),
];

/// Most filter drivers emptied; a configuration with more is refused.
const MAX_DRIVERS: usize = 64;

/// The git binary the host runs, resolved once: from a directory only root can write when
/// there is one, so a `git` an agent planted on the service's `PATH` (such as `~/.local/bin`,
/// which the open profile can write) is not the one the host runs.
pub(crate) fn program() -> &'static Path {
    static GIT: OnceLock<PathBuf> = OnceLock::new();
    GIT.get_or_init(|| hq_sandbox::trusted_program("git").unwrap_or_else(|| PathBuf::from("git")))
}

/// The bare command, without the repository's own pins. Every host git call goes through here.
fn base(dir: &Path) -> Command {
    let mut cmd = Command::new(program());
    cmd.arg("--no-pager").arg("-C").arg(dir);
    for (k, v) in PINS {
        cmd.arg("-c").arg(format!("{k}={v}"));
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PAGER", "cat")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_EXTERNAL_DIFF")
        .stdin(Stdio::null());
    cmd
}

/// The filter drivers the configuration seen from `dir` names. `None` when git cannot say.
fn filter_drivers(dir: &Path) -> Option<Vec<String>> {
    let out = base(dir)
        .args([
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            r"^filter\..+\.(clean|smudge|process|required)$",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    // Exit 1 means no key matched.
    match out.status.code() {
        Some(0) | Some(1) => {}
        _ => return None,
    }
    let mut names: Vec<String> = Vec::new();
    for key in out.stdout.split(|b| *b == 0).filter(|k| !k.is_empty()) {
        let key = String::from_utf8_lossy(key);
        let Some(rest) = key.strip_prefix("filter.") else {
            continue;
        };
        let Some((name, _)) = rest.rsplit_once('.') else {
            continue;
        };
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Some(names)
}

/// Git in `dir`, with every repository-named command off. `None` when the repository's filter
/// drivers cannot be read or are too many to pin, so the caller runs nothing.
pub(crate) fn command(dir: &Path) -> Option<Command> {
    let drivers = filter_drivers(dir)?;
    if drivers.len() > MAX_DRIVERS {
        return None;
    }
    let mut cmd = base(dir);
    // Through the environment rather than `-c`, so a driver name holding `=` is still one key.
    let mut n = 0usize;
    for name in &drivers {
        for (key, value) in FILTER_KEYS {
            cmd.env(
                format!("GIT_CONFIG_KEY_{n}"),
                format!("filter.{name}.{key}"),
            )
            .env(format!("GIT_CONFIG_VALUE_{n}"), value);
            n += 1;
        }
    }
    cmd.env("GIT_CONFIG_COUNT", n.to_string());
    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{self, Base};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository whose configuration names a command at every place git would run one for
    /// the host: fsmonitor, a post-checkout hook, a hooks path, clean and smudge filters (one
    /// of them in a driver whose name holds `=`, one `required`), a long-running filter process,
    /// and `gpg.program` behind `log.showSignature` with a signed head. Each appends to `marker`.
    fn hostile() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("marker");
        let evil = tmp.path().join("evil.sh");
        fs::write(
            &evil,
            format!(
                "#!/bin/sh\necho \"$0 $*\" >> '{}'\ncat\nexit 0\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&evil, fs::Permissions::from_mode(0o755)).unwrap();
        let e = evil.display().to_string();

        let repo = tmp.path().join("app");
        fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q", "-b", "main"]);
        fs::write(
            repo.join(".gitattributes"),
            "*.txt filter=evil\n*.md filter=a=b\n*.rs filter=proc\n",
        )
        .unwrap();
        fs::write(repo.join("a.txt"), "one\n").unwrap();
        fs::write(repo.join("b.md"), "two\n").unwrap();
        fs::write(repo.join("c.rs"), "three\n").unwrap();
        fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        fs::write(repo.join(".worktreeinclude"), ".env\n").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-q", "-m", "first"]);
        fs::write(repo.join(".env"), "KEY=1").unwrap();

        // A head commit that carries a signature, so `log.showSignature` would call gpg.
        let tree = run(&repo, &["rev-parse", "HEAD^{tree}"]);
        let parent = run(&repo, &["rev-parse", "HEAD"]);
        let body = format!(
            "tree {tree}\nparent {parent}\nauthor t <t@example.com> 0 +0000\n\
             committer t <t@example.com> 0 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n \
             AAAA\n -----END PGP SIGNATURE-----\n\nsigned\n"
        );
        let obj = tmp.path().join("commit.txt");
        fs::write(&obj, body).unwrap();
        let sha = run(
            &repo,
            &["hash-object", "-t", "commit", "-w", obj.to_str().unwrap()],
        );
        run(&repo, &["update-ref", "refs/heads/main", &sha]);

        // Only now, so setting the repository up ran none of it.
        let hooks = tmp.path().join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        for hook in [
            "post-checkout",
            "reference-transaction",
            "post-index-change",
        ] {
            fs::copy(&evil, repo.join(".git/hooks").join(hook)).unwrap();
            fs::copy(&evil, hooks.join(hook)).unwrap();
        }
        for (k, v) in [
            ("core.fsmonitor", e.clone()),
            ("core.hooksPath", hooks.display().to_string()),
            ("filter.evil.clean", format!("{e} clean")),
            ("filter.evil.smudge", format!("{e} smudge")),
            ("filter.evil.required", "true".into()),
            ("filter.a=b.clean", format!("{e} clean-eq")),
            ("filter.a=b.smudge", format!("{e} smudge-eq")),
            ("filter.proc.process", format!("{e} process")),
            ("log.showSignature", "true".into()),
            ("gpg.program", e.clone()),
            ("core.sshCommand", e.clone()),
        ] {
            run(&repo, &["config", k, &v]);
        }
        let root = tmp.path().join("HQ");
        fs::create_dir_all(&root).unwrap();
        (tmp, repo, root, marker)
    }

    /// What ran since the last look, by step; the marker is emptied each time.
    fn ran(marker: &Path, step: &str, all: &mut Vec<String>) {
        let text = fs::read_to_string(marker).unwrap_or_default();
        let _ = fs::remove_file(marker);
        for line in text.lines() {
            all.push(format!("{step}: {line}"));
        }
    }

    #[test]
    fn host_git_runs_nothing_the_repository_names() {
        let (_tmp, repo, root, marker) = hostile();
        let mut all = Vec::new();
        let head = worktree::head(&repo).expect("head");
        ran(&marker, "head", &mut all);

        let wt = worktree::add(
            &root,
            &repo,
            "hs-1",
            &Base::Branch {
                name: "work".into(),
                from: None,
            },
        )
        .expect("worktree add");
        ran(&marker, "worktree add", &mut all);
        // A stat change makes status compare contents, which runs the clean filter.
        let old = fs::File::options()
            .write(true)
            .open(wt.path.join("a.txt"))
            .unwrap();
        old.set_modified(std::time::SystemTime::UNIX_EPOCH).unwrap();
        drop(old);
        let s = worktree::status(&root, &wt.path, Some(&head.sha)).expect("status");
        ran(&marker, "status", &mut all);
        let _ = worktree::head(&wt.path).expect("head of the worktree");
        ran(&marker, "head of the worktree", &mut all);
        let gone = worktree::remove(&root, &wt.path).expect("remove");
        ran(&marker, "remove", &mut all);

        assert!(
            all.is_empty(),
            "host git ran repository commands:\n{}",
            all.join("\n")
        );
        assert_eq!(head.subject, "signed");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
        assert_eq!(wt.copied, [".env"], "{wt:?}");
        assert_eq!(s.changed, 0, "{s:?}");
        assert!(gone.removed, "{gone:?}");
    }

    #[test]
    fn the_pins_beat_the_repository() {
        let (_tmp, repo, _root, _marker) = hostile();
        let read = |key: &str| {
            let out = command(&repo)
                .unwrap()
                .args(["config", "--get", key])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(read("core.fsmonitor"), "false");
        assert_eq!(read("core.hooksPath"), "/dev/null");
        assert_eq!(read("protocol.allow"), "never");
        assert_eq!(read("filter.evil.clean"), "");
        assert_eq!(read("filter.a=b.smudge"), "");
        assert_eq!(read("filter.proc.process"), "");
        assert_eq!(read("filter.evil.required"), "false");
    }

    /// A `git` early on `PATH` in a directory the user owns is passed over for the system's.
    #[test]
    fn host_git_is_the_systems_and_not_the_first_on_path() {
        let git = program();
        if Path::new("/usr/bin/git").is_file() {
            assert!(git.is_absolute(), "{}", git.display());
            if let Some(home) = std::env::var_os("HOME").filter(|h| h != "/") {
                assert!(!git.starts_with(home), "{}", git.display());
            }
        }
    }
}
