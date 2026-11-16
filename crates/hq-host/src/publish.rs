//! Publishing a helper's committed work: the host pushes the branch and opens the pull request
//! with its own login, so the sandboxed agent never holds a credential. Everything is checked
//! mechanically first and a refusal says exactly why. Never merges, approves, forces, deletes a
//! ref or pushes a tag. The pull request is opened by `gh` run from a folder that is not the
//! agent's repository, so nothing the agent wrote into that repository's configuration runs.

use crate::worktree::{WORKTREES_DIR, WorktreeError, check_ours, current_branch, git};
use hq_core::publish_check::{
    MAX_BODY_CHARS, MAX_TITLE_CHARS, check_branch, env_secrets, holds_secret, is_protected_branch,
    refused_path, repo_slug_of_url,
};
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const REMOTE: &str = "origin";
const MODE_SUBMODULE: &str = "160000";
const STATUS_DELETED: &str = "D";
/// Fallback default branches when the remote's HEAD was never recorded locally.
const COMMON_DEFAULTS: [&str; 2] = ["main", "master"];
/// Blobs a push may carry in total across its commits, so a check always finishes.
const MAX_HISTORY_BLOBS: usize = 5000;

/// What to publish, and the caps it must stay under.
pub struct Request<'a> {
    pub path: &'a Path,
    pub branch: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    /// The repository the caller expects `origin` to be. Required unless `check_only`.
    pub repo_slug: Option<&'a str>,
    /// Run every check and report, push nothing.
    pub check_only: bool,
    pub max_files: usize,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub repo: String,
    pub branch: String,
    pub base: String,
    pub files: usize,
    pub bytes: u64,
    pub commits: u32,
    /// The pull request, once opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The pull request already existed for this branch.
    pub existing: bool,
}

struct Change {
    status: String,
    mode: String,
    sha: String,
    path: String,
}

fn refuse<T>(why: impl Into<String>) -> Result<T, WorktreeError> {
    Err(WorktreeError::Refused(why.into()))
}

/// [`publish`] with the host's own `gh`.
pub fn publish(root: &Path, req: &Request) -> Result<Report, WorktreeError> {
    publish_with(root, req, hq_sandbox::trusted_program("gh").as_deref())
}

pub fn publish_with(
    root: &Path,
    req: &Request,
    gh: Option<&Path>,
) -> Result<Report, WorktreeError> {
    check_text(req)?;
    let holder = root.join(WORKTREES_DIR).canonicalize()?;
    let path = req.path.canonicalize()?;
    check_ours(&holder, &path)?;
    check_branch(req.branch).map_err(|e| WorktreeError::Refused(e.into()))?;
    if current_branch(&path).as_deref() != Some(req.branch) {
        return refuse(format!(
            "the worktree is not on branch {}; publish the branch it has checked out",
            req.branch
        ));
    }
    let slug = origin_slug(&path, req)?;
    let default = default_branch(&path)?;
    if is_protected_branch(req.branch, Some(&default)) {
        return refuse(format!(
            "{} is a protected branch; work on a branch of its own",
            req.branch
        ));
    }
    if !git(&path, &["status", "--porcelain"])?.is_empty() {
        return refuse("there are uncommitted changes; commit them first");
    }
    let base = format!("{REMOTE}/{default}");
    let commits = commits_ahead(&path, &base)?;
    let changes = changes_since(&path, &base)?;
    let bytes = check_changes(&path, &changes, req)?;
    check_history(&path, &base, req)?;
    let head = git(&path, &["rev-parse", "HEAD"])?;
    let mut report = Report {
        repo: slug.clone(),
        branch: req.branch.into(),
        base: default.clone(),
        files: changes.len(),
        bytes,
        commits,
        url: None,
        existing: false,
    };
    if req.check_only {
        return Ok(report);
    }
    push(&path, &head, req.branch)?;
    let (url, existing) = open_pr(gh, root, &slug, &default, req)?;
    report.url = Some(url);
    report.existing = existing;
    Ok(report)
}

fn check_text(req: &Request) -> Result<(), WorktreeError> {
    if req.title.trim().is_empty() && !req.check_only {
        return refuse("a title is required");
    }
    if req.title.chars().count() > MAX_TITLE_CHARS || req.body.chars().count() > MAX_BODY_CHARS {
        return refuse(format!(
            "the title may be {MAX_TITLE_CHARS} characters and the body {MAX_BODY_CHARS}"
        ));
    }
    let secrets = env_secrets();
    for (name, text) in [
        ("title", req.title),
        ("body", req.body),
        ("branch", req.branch),
    ] {
        if holds_secret(text, &secrets) {
            return refuse(format!("`{name}` holds something shaped like a credential"));
        }
    }
    Ok(())
}

/// `owner/name` of `origin`, which must be `expected` when the caller named one, and which no
/// repository setting may redirect a push away from.
fn origin_slug(path: &Path, req: &Request) -> Result<String, WorktreeError> {
    match crate::host_git::push_redirects(path) {
        Some(keys) if keys.is_empty() => {}
        Some(keys) => {
            return refuse(format!(
                "the repository's own configuration redirects pushes ({}); remove it",
                keys.join(", ")
            ));
        }
        None => return refuse("git could not read the repository's configuration"),
    }
    let url = git(path, &["config", "--get", &format!("remote.{REMOTE}.url")]).map_err(|_| {
        WorktreeError::Refused(format!("the repository has no remote named {REMOTE}"))
    })?;
    let Some(slug) = repo_slug_of_url(&url) else {
        return refuse("the remote's address does not name an owner and repository");
    };
    match req.repo_slug {
        Some(want) if !want.eq_ignore_ascii_case(&slug) => {
            refuse(format!("origin is {slug}, not {want}"))
        }
        None if !req.check_only => refuse("repo_slug is required"),
        _ => Ok(slug),
    }
}

fn default_branch(path: &Path) -> Result<String, WorktreeError> {
    let prefix = format!("{REMOTE}/");
    if let Ok(head) = git(
        path,
        &[
            "symbolic-ref",
            "--short",
            "-q",
            &format!("refs/remotes/{REMOTE}/HEAD"),
        ],
    ) && let Some(name) = head.strip_prefix(&prefix)
    {
        return Ok(name.to_string());
    }
    for name in COMMON_DEFAULTS {
        if git(
            path,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{REMOTE}/{name}"),
            ],
        )
        .is_ok()
        {
            return Ok(name.to_string());
        }
    }
    refuse(
        "cannot tell the repository's default branch; fetch it first (git remote set-head origin -a)",
    )
}

fn commits_ahead(path: &Path, base: &str) -> Result<u32, WorktreeError> {
    let n = git(path, &["rev-list", "--count", &format!("{base}..HEAD")])?;
    match n.parse::<u32>() {
        Ok(0) => refuse(format!("no commits ahead of {base}; nothing to publish")),
        Ok(n) => Ok(n),
        Err(_) => refuse("could not count the commits ahead of the base"),
    }
}

/// The files changed from where this branch left the base, as git's raw diff lists them.
fn changes_since(path: &Path, base: &str) -> Result<Vec<Change>, WorktreeError> {
    let fork = git(path, &["merge-base", base, "HEAD"])
        .map_err(|_| WorktreeError::Refused(format!("{base} and this branch share no history")))?;
    let raw = git(
        path,
        &[
            "diff",
            "--raw",
            "-z",
            "--no-renames",
            "--no-textconv",
            "--no-ext-diff",
            &fork,
            "HEAD",
        ],
    )?;
    parse_raw(&raw)
}

/// Git's raw diff output (`-z`) as a list of changes.
fn parse_raw(raw: &str) -> Result<Vec<Change>, WorktreeError> {
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    let mut out = Vec::new();
    while let Some(meta) = tokens.next() {
        let Some(file) = tokens.next() else { break };
        let fields: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        let [_, mode, _, sha, status] = fields[..] else {
            return refuse("could not read the list of changed files");
        };
        out.push(Change {
            status: status.chars().take(1).collect(),
            mode: mode.into(),
            sha: sha.into(),
            path: file.into(),
        });
    }
    Ok(out)
}

/// Refuses on the first path, size, count or content that must not be pushed. Returns the bytes
/// the changes add up to.
fn check_changes(path: &Path, changes: &[Change], req: &Request) -> Result<u64, WorktreeError> {
    if changes.is_empty() {
        return refuse("the branch changes no files compared with its base");
    }
    if changes.len() > req.max_files {
        return refuse(format!(
            "{} files changed, more than the {} one publish may carry; publish smaller pieces",
            changes.len(),
            req.max_files
        ));
    }
    for c in changes {
        if let Some(why) = refused_path(&c.path) {
            return refuse(format!("{} is never published: {why}", c.path));
        }
        if c.mode == MODE_SUBMODULE {
            return refuse(format!(
                "{} is a submodule pointer; HQ does not publish those",
                c.path
            ));
        }
    }
    let secrets = env_secrets();
    let mut total = 0u64;
    for c in changes.iter().filter(|c| c.status != STATUS_DELETED) {
        let size = blob_size(path, &c.sha)?;
        if size > req.max_bytes {
            return refuse(format!(
                "{} is {size} bytes, over the {} byte limit",
                c.path, req.max_bytes
            ));
        }
        total += size;
        if holds_secret(&blob_text(path, &c.sha)?, &secrets) {
            return refuse(format!(
                "{} holds something shaped like a credential",
                c.path
            ));
        }
    }
    Ok(total)
}

/// Every file version in every commit a push would carry, whatever the final diff shows: a
/// credential added in one commit and removed in the next is still in the history that goes up.
fn check_history(path: &Path, base: &str, req: &Request) -> Result<(), WorktreeError> {
    let range = format!("{base}..HEAD");
    if git(path, &["rev-list", "--merges", "--count", &range])? != "0" {
        return refuse("the branch holds a merge commit; rebase it onto the base first");
    }
    let secrets = env_secrets();
    let mut seen = 0usize;
    for commit in git(path, &["rev-list", &range])?.lines() {
        let raw = git(
            path,
            &[
                "diff-tree",
                "--root",
                "-r",
                "--raw",
                "-z",
                "--no-renames",
                "--no-textconv",
                "--no-commit-id",
                commit,
            ],
        )?;
        for c in parse_raw(&raw)?
            .iter()
            .filter(|c| c.status != STATUS_DELETED)
        {
            seen += 1;
            if seen > MAX_HISTORY_BLOBS {
                return refuse("the commits carry too many files to check; publish smaller pieces");
            }
            if let Some(why) = refused_path(&c.path) {
                return refuse(format!(
                    "{} is in the history and is never published: {why}",
                    c.path
                ));
            }
            if blob_size(path, &c.sha)? > req.max_bytes {
                return refuse(format!(
                    "{} is over the {} byte limit in the history",
                    c.path, req.max_bytes
                ));
            }
            if holds_secret(&blob_text(path, &c.sha)?, &secrets) {
                return refuse(format!(
                    "{} held something shaped like a credential in an earlier commit; squash the history",
                    c.path
                ));
            }
        }
    }
    Ok(())
}

fn blob_size(path: &Path, sha: &str) -> Result<u64, WorktreeError> {
    git(path, &["cat-file", "-s", sha])?
        .parse()
        .map_err(|_| WorktreeError::Git("could not size a changed file".into()))
}

/// The blob as text, without any filter running on it.
fn blob_text(path: &Path, sha: &str) -> Result<String, WorktreeError> {
    git(path, &["cat-file", "blob", sha])
}

/// Pushes the commit that was checked, not whatever `HEAD` is by now: the agent keeps running.
/// Never forced, no tags, no deletion.
fn push(path: &Path, head: &str, branch: &str) -> Result<(), WorktreeError> {
    let Some(mut cmd) = crate::host_git::for_push(path) else {
        return refuse("the repository's own configuration names a program a push would run");
    };
    let refspec = format!("{head}:refs/heads/{branch}");
    let out = cmd
        .args(["push", "--no-follow-tags", "--no-verify", REMOTE, &refspec])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| WorktreeError::Step("push_failed", format!("git could not run: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git push failed");
    Err(WorktreeError::Step("push_failed", line.trim().to_string()))
}

/// Opens the pull request, or finds the one already open for the branch.
fn open_pr(
    gh: Option<&Path>,
    root: &Path,
    slug: &str,
    base: &str,
    req: &Request,
) -> Result<(String, bool), WorktreeError> {
    let Some(gh) = gh else {
        return Err(WorktreeError::Step(
            "gh_missing",
            "the GitHub CLI (gh) is not installed on the host".into(),
        ));
    };
    let gh_cmd = || {
        let mut cmd = Command::new(gh);
        cmd.current_dir(root)
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null());
        cmd
    };
    let auth = gh_cmd().args(["auth", "status"]).output();
    if !auth.is_ok_and(|o| o.status.success()) {
        return Err(WorktreeError::Step(
            "gh_not_logged_in",
            "the host's gh is not logged in".into(),
        ));
    }
    let mut cmd = gh_cmd();
    cmd.args([
        "pr", "create", "--repo", slug, "--base", base, "--head", req.branch,
    ])
    .args(["--title", req.title, "--body-file", "-"])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| WorktreeError::Step("gh_failed", format!("gh could not run: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(req.body.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| WorktreeError::Step("gh_failed", e.to_string()))?;
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    if out.status.success() {
        return pr_url(&stdout).map(|u| (u, false)).ok_or_else(|| {
            WorktreeError::Step("gh_failed", "gh printed no pull request address".into())
        });
    }
    // gh names the open pull request when the branch already has one.
    match stderr
        .contains("already exists")
        .then(|| pr_url(&stderr))
        .flatten()
    {
        Some(url) => Ok((url, true)),
        None => Err(WorktreeError::Step("gh_failed", last_line(&stderr))),
    }
}

fn pr_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(str::to_string)
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("gh failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{Base, add};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    const PR_URL: &str = "https://example.com/acme/widgets/pull/7";
    const MAX_FILES: usize = 20;
    const MAX_BYTES: u64 = 4096;

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

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        origin: PathBuf,
        worktree: PathBuf,
        gh: PathBuf,
        gh_dir: PathBuf,
    }

    /// A bare `acme/widgets.git` as origin, a clone with `main` pushed, and a worktree of it on
    /// `feat/x`. The stub `gh` logs its stdin and prints a pull request address.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("acme").join("widgets.git");
        fs::create_dir_all(&origin).unwrap();
        run(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let work = tmp.path().join("work");
        run(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "work"],
        );
        run(&work, &["checkout", "-q", "-b", "main"]);
        fs::write(work.join("README.md"), "hello\n").unwrap();
        run(&work, &["add", "-A"]);
        run(&work, &["commit", "-q", "-m", "first"]);
        run(&work, &["push", "-q", "origin", "main"]);
        run(&work, &["remote", "set-head", "origin", "main"]);
        let root = tmp.path().join("HQ");
        fs::create_dir_all(&root).unwrap();
        let wt = add(
            &root,
            &work,
            "s1",
            &Base::Branch {
                name: "feat/x".into(),
                from: None,
            },
        )
        .unwrap();
        let gh_dir = tmp.path().join("gh-state");
        fs::create_dir_all(&gh_dir).unwrap();
        let gh = tmp.path().join("gh");
        fs::write(
            &gh,
            format!(
                "#!/bin/sh\nd='{d}'\nif [ \"$1 $2\" = \"auth status\" ]; then [ -e \"$d/noauth\" ] && exit 1; exit 0; fi\n\
                 cat > \"$d/body\"\necho \"$@\" > \"$d/args\"\necho '{PR_URL}'\n",
                d = gh_dir.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        Fixture {
            _tmp: tmp,
            root,
            origin,
            worktree: wt.path,
            gh,
            gh_dir,
        }
    }

    impl Fixture {
        fn commit(&self, file: &str, content: &str) {
            let p = self.worktree.join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
            run(&self.worktree, &["add", "-A"]);
            run(&self.worktree, &["commit", "-q", "-m", "work"]);
        }

        fn req<'a>(&'a self, branch: &'a str) -> Request<'a> {
            Request {
                path: &self.worktree,
                branch,
                title: "Add things",
                body: "Because.",
                repo_slug: Some("acme/widgets"),
                check_only: false,
                max_files: MAX_FILES,
                max_bytes: MAX_BYTES,
            }
        }

        fn publish(&self, req: &Request) -> Result<Report, WorktreeError> {
            publish_with(&self.root, req, Some(&self.gh))
        }

        fn remote_has(&self, branch: &str) -> bool {
            Command::new("git")
                .arg("-C")
                .arg(&self.origin)
                .args([
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/heads/{branch}"),
                ])
                .output()
                .unwrap()
                .status
                .success()
        }
    }

    fn refusal(r: Result<Report, WorktreeError>) -> String {
        match r {
            Err(WorktreeError::Refused(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn committed_work_is_pushed_and_a_pull_request_opened_with_the_body_on_stdin() {
        let f = fixture();
        f.commit("src/a.txt", "one\n");
        let report = f.publish(&f.req("feat/x")).unwrap();
        assert_eq!(report.url.as_deref(), Some(PR_URL));
        assert_eq!(
            (report.repo.as_str(), report.base.as_str()),
            ("acme/widgets", "main")
        );
        assert_eq!((report.files, report.commits), (1, 1));
        assert!(f.remote_has("feat/x"));
        assert_eq!(
            fs::read_to_string(f.gh_dir.join("body")).unwrap(),
            "Because."
        );
        let args = fs::read_to_string(f.gh_dir.join("args")).unwrap();
        assert!(
            args.contains("--head feat/x") && args.contains("--repo acme/widgets"),
            "{args}"
        );
        assert!(
            !args.contains("merge") && !args.contains("approve"),
            "{args}"
        );
    }

    #[test]
    fn a_check_only_request_reports_the_repository_and_pushes_nothing() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        let mut req = f.req("feat/x");
        req.check_only = true;
        req.repo_slug = None;
        let report = f.publish(&req).unwrap();
        assert_eq!(report.repo, "acme/widgets");
        assert!(report.url.is_none() && !f.remote_has("feat/x"));
    }

    #[test]
    fn a_real_publish_without_a_repository_name_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        let mut req = f.req("feat/x");
        req.repo_slug = None;
        assert!(refusal(f.publish(&req)).contains("repo_slug"));
    }

    #[test]
    fn the_default_branch_is_never_published() {
        let f = fixture();
        run(&f.worktree, &["checkout", "-q", "-b", "main2"]);
        f.commit("a.txt", "one\n");
        let msg = refusal(f.publish(&f.req("main")));
        assert!(msg.contains("not on branch main"), "{msg}");
        run(&f.worktree, &["branch", "-q", "-m", "release/1"]);
        assert!(refusal(f.publish(&f.req("release/1"))).contains("protected"));
        assert!(!f.remote_has("release/1"));
    }

    #[test]
    fn a_branch_other_than_the_checked_out_one_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        assert!(refusal(f.publish(&f.req("feat/y"))).contains("not on branch"));
    }

    #[test]
    fn a_detached_head_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        run(&f.worktree, &["checkout", "-q", "--detach"]);
        assert!(refusal(f.publish(&f.req("feat/x"))).contains("not on branch"));
    }

    #[test]
    fn uncommitted_changes_are_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        fs::write(f.worktree.join("dirty.txt"), "x").unwrap();
        assert!(refusal(f.publish(&f.req("feat/x"))).contains("uncommitted"));
    }

    #[test]
    fn a_branch_with_no_commits_ahead_is_refused() {
        let f = fixture();
        assert!(refusal(f.publish(&f.req("feat/x"))).contains("no commits ahead"));
    }

    #[test]
    fn workflow_hook_and_git_files_are_refused() {
        for bad in [
            ".github/workflows/ci.yml",
            ".husky/pre-commit",
            ".gitattributes",
        ] {
            let f = fixture();
            f.commit("ok.txt", "fine\n");
            f.commit(bad, "x\n");
            let msg = refusal(f.publish(&f.req("feat/x")));
            assert!(
                msg.contains(bad) && msg.contains("never published"),
                "{bad}: {msg}"
            );
            assert!(!f.remote_has("feat/x"));
        }
    }

    #[test]
    fn a_file_shaped_like_a_credential_is_refused() {
        let f = fixture();
        f.commit(
            "notes.txt",
            "token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789\n",
        );
        let msg = refusal(f.publish(&f.req("feat/x")));
        assert!(msg.contains("credential"), "{msg}");
        assert!(!f.remote_has("feat/x"));
    }

    #[test]
    fn a_credential_added_and_removed_in_later_commits_is_still_refused() {
        let f = fixture();
        f.commit(
            "notes.txt",
            "token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789\n",
        );
        f.commit("notes.txt", "clean now\n");
        let msg = refusal(f.publish(&f.req("feat/x")));
        assert!(msg.contains("earlier commit"), "{msg}");
        assert!(!f.remote_has("feat/x"));
    }

    #[test]
    fn a_title_or_body_shaped_like_a_credential_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        let mut req = f.req("feat/x");
        req.body = "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(refusal(f.publish(&req)).contains("`body`"));
    }

    #[test]
    fn too_many_or_too_large_changes_are_refused() {
        let f = fixture();
        f.commit("big.txt", &"x".repeat(MAX_BYTES as usize + 1));
        assert!(refusal(f.publish(&f.req("feat/x"))).contains("over the"));
        let f = fixture();
        f.commit("a.txt", "1");
        f.commit("b.txt", "2");
        let mut req = f.req("feat/x");
        req.max_files = 1;
        assert!(refusal(f.publish(&req)).contains("2 files changed"));
    }

    #[test]
    fn a_remote_that_is_not_the_named_repository_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        let mut req = f.req("feat/x");
        req.repo_slug = Some("acme/other");
        assert!(refusal(f.publish(&req)).contains("not acme/other"));
    }

    #[test]
    fn a_repository_that_redirects_pushes_is_refused() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        run(
            &f.worktree,
            &[
                "config",
                "remote.origin.pushurl",
                "https://example.com/evil/repo.git",
            ],
        );
        assert!(refusal(f.publish(&f.req("feat/x"))).contains("redirects pushes"));
    }

    #[test]
    fn a_diverged_remote_branch_is_never_overwritten() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        f.publish(&f.req("feat/x")).unwrap();
        let theirs = run(&f.origin, &["rev-parse", "refs/heads/feat/x"]);
        run(&f.worktree, &["reset", "-q", "--hard", "HEAD~1"]);
        f.commit("b.txt", "other\n");
        let err = f.publish(&f.req("feat/x")).unwrap_err();
        assert_eq!(err.code(), "push_failed");
        assert_eq!(run(&f.origin, &["rev-parse", "refs/heads/feat/x"]), theirs);
    }

    #[test]
    fn a_missing_or_logged_out_gh_is_a_plain_code_after_the_push_checks() {
        let f = fixture();
        f.commit("a.txt", "one\n");
        let err = publish_with(&f.root, &f.req("feat/x"), None).unwrap_err();
        assert_eq!(err.code(), "gh_missing");
        fs::write(f.gh_dir.join("noauth"), "").unwrap();
        let err = f.publish(&f.req("feat/x")).unwrap_err();
        assert_eq!(err.code(), "gh_not_logged_in");
    }

    #[test]
    fn a_path_outside_the_hosts_worktrees_is_refused() {
        let f = fixture();
        let mut req = f.req("feat/x");
        req.path = &f.origin;
        assert!(matches!(f.publish(&req), Err(WorktreeError::Invalid(_))));
    }
}
