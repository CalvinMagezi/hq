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

/// Signing programs (`commit.gpgSign` runs them) and what each falls back to when the
/// repository's own configuration names one. A value from the owner's global configuration
/// (such as a password manager's ssh signer) is left alone.
const LOCAL_SIGNING_PROGRAMS: &[(&str, &str)] = &[
    ("gpg.program", "gpg"),
    ("gpg.ssh.program", "ssh-keygen"),
    ("gpg.x509.program", "gpgsm"),
    ("gpg.ssh.defaultkeycommand", ""),
];

/// The one pin a push cannot keep: it needs a transport. `ext::` stays closed, since it runs
/// whatever command the URL names.
const TRANSPORT_PIN: &str = "protocol.allow";

/// Keys in a repository's own configuration that make a push run a program. A push from a
/// repository that sets one is refused rather than run (see [`for_push`]).
const PUSH_PROGRAM_KEYS: &str = r"^(core\.sshcommand|core\.gitproxy|core\.askpass|credential\..*|remote\..*\.(receivepack|uploadpack|vcs))$";

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
    base_with(dir, false)
}

fn base_with(dir: &Path, transport: bool) -> Command {
    let mut cmd = Command::new(program());
    cmd.arg("--no-pager").arg("-C").arg(dir);
    for (k, v) in PINS {
        if transport && *k == TRANSPORT_PIN {
            continue;
        }
        cmd.arg("-c").arg(format!("{k}={v}"));
    }
    if transport {
        cmd.arg("-c").arg("protocol.ext.allow=never");
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
    pinned(dir, false)
}

/// [`command`] for HQ's own git tools, which run git in the session's repositories outside any
/// sandbox, the same exposure the host has.
pub fn hardened(dir: &Path) -> Option<Command> {
    pinned(dir, false)
}

/// [`hardened`] with a transport, for a push. `None` as well when the repository's own
/// configuration names a program the push would run (an ssh command, a proxy, an askpass or
/// credential helper, a remote's receive-pack): a global setting the owner made still applies.
pub fn for_push(dir: &Path) -> Option<Command> {
    if !repository_keys(dir, PUSH_PROGRAM_KEYS)?.is_empty() {
        return None;
    }
    pinned(dir, true)
}

/// Keys matching `pattern` that the repository itself sets (its `.git/config` or a worktree's
/// config, both of which an agent working in it can write), lowercased. Works inside or outside
/// a repository; `None` when git cannot say.
fn repository_keys(dir: &Path, pattern: &str) -> Option<Vec<String>> {
    let out = base(dir)
        .args([
            "config",
            "--show-scope",
            "--name-only",
            "--get-regexp",
            pattern,
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
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter(|(scope, _)| matches!(*scope, "local" | "worktree"))
            .map(|(_, key)| key.trim().to_ascii_lowercase())
            .collect(),
    )
}

/// The signing-program keys the repository's own configuration sets. `None` when git cannot say.
fn local_signing_programs(dir: &Path) -> Option<Vec<&'static (&'static str, &'static str)>> {
    let keys = repository_keys(
        dir,
        r"^gpg\.(program|ssh\.program|x509\.program|ssh\.defaultkeycommand)$",
    )?;
    Some(
        LOCAL_SIGNING_PROGRAMS
            .iter()
            .filter(|(k, _)| keys.iter().any(|key| key == k))
            .collect(),
    )
}

fn pinned(dir: &Path, transport: bool) -> Option<Command> {
    let drivers = filter_drivers(dir)?;
    if drivers.len() > MAX_DRIVERS {
        return None;
    }
    let signing = local_signing_programs(dir)?;
    let mut cmd = base_with(dir, transport);
    for (key, fallback) in signing {
        cmd.arg("-c").arg(format!("{key}={fallback}"));
    }
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

    /// HQ's own git tools commit and diff in the session's repository, outside any sandbox: a
    /// pre-commit hook, a signing program or a diff driver the repository names must not run.
    #[test]
    fn hardened_git_commits_and_diffs_without_running_what_the_repository_names() {
        let (tmp, repo, _root, marker) = hostile();
        let e = tmp.path().join("evil.sh").display().to_string();
        fs::copy(&e, repo.join(".git/hooks/pre-commit")).unwrap();
        fs::copy(&e, tmp.path().join("hooks/pre-commit")).unwrap();
        for (k, v) in [
            ("commit.gpgSign", "true".to_string()),
            ("diff.evil.textconv", e.clone()),
            ("diff.evil.command", e.clone()),
        ] {
            run(&repo, &["config", k, &v]);
        }
        fs::write(repo.join(".gitattributes"), "*.txt filter=evil diff=evil\n").unwrap();
        fs::write(repo.join("a.txt"), "changed\n").unwrap();
        let _ = fs::remove_file(&marker);
        let mut all = Vec::new();

        let git = |args: &[&str]| {
            let out = hardened(&repo)
                .expect("hardened git")
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let diff = git(&["diff", "--no-ext-diff", "--no-textconv"]);
        ran(&marker, "diff", &mut all);
        git(&["add", "-A"]);
        ran(&marker, "add", &mut all);
        // Unsigned first, so the commit itself must succeed: hooks and filters stay off.
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-q",
            "-m",
            "x",
        ]);
        ran(&marker, "commit", &mut all);
        // Signed: the repository's gpg.program is replaced by the standard one, which may have no
        // key on this machine and fail. Only that the repository's program never ran matters.
        fs::write(repo.join("a.txt"), "again\n").unwrap();
        let _ = hardened(&repo)
            .expect("hardened git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["commit", "-q", "-a", "-m", "signed"])
            .output()
            .unwrap();
        ran(&marker, "signed commit", &mut all);
        assert!(
            all.is_empty(),
            "ran repository commands:\n{}",
            all.join("\n")
        );
        assert!(diff.contains("changed"), "{diff}");
    }

    /// A push needs a transport, so it cannot keep every pin; it refuses a repository whose own
    /// configuration names a program the push would run instead.
    #[test]
    fn a_push_refuses_a_repository_that_names_a_program_and_allows_a_plain_one() {
        let (_tmp, repo, _root, _marker) = hostile();
        assert!(
            for_push(&repo).is_none(),
            "core.sshCommand is set in this repository"
        );
        let plain = tempfile::tempdir().unwrap();
        run(plain.path(), &["init", "-q", "-b", "main"]);
        let cmd = for_push(plain.path()).expect("a plain repository may push");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            !args.iter().any(|a| a == "protocol.allow=never"),
            "{args:?}"
        );
        assert!(
            args.iter().any(|a| a == "protocol.ext.allow=never"),
            "{args:?}"
        );
        assert!(
            args.iter().any(|a| a == "core.hooksPath=/dev/null"),
            "{args:?}"
        );
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
