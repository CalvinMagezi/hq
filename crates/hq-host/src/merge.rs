//! Merging a helper's pull request, with the host's own `gh` login, only for the exact commit the
//! owner approved. The host re-reads the pull request every time, so a commit pushed or a check
//! turned red after the approval stops the merge. Squash only, never forced, never an admin
//! override, never a repository or branch the rules in `hq_core::merge_check` refuse.
//! `gh` runs from the HQ folder with `--repo`, so no repository configuration is read.

use crate::worktree::WorktreeError;
use hq_core::merge_check::{PrFacts, facts_from_view, judge};
use hq_core::publish_check::repo_slug_of_url;
use serde::Serialize;
use std::path::Path;
use std::process::{Command, Stdio};

const VIEW_FIELDS: &str = "number,title,url,state,isDraft,isCrossRepository,baseRefName,headRefName,headRefOid,mergeable,statusCheckRollup";
/// Git object names are 40 hex digits (SHA-1) or 64 (SHA-256).
const SHA_LENGTHS: [usize; 2] = [40, 64];

/// Which pull request, and the commit the owner approved. Without `approved_sha` nothing is
/// merged: the host only reports whether the pull request is ready.
pub struct Request<'a> {
    pub repo: &'a str,
    pub number: u64,
    pub approved_sha: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head_sha: String,
    pub base: String,
    pub head_ref: String,
    /// Names of the checks that passed.
    pub checks: Vec<String>,
    pub merged: bool,
}

fn refuse<T>(why: impl Into<String>) -> Result<T, WorktreeError> {
    Err(WorktreeError::Refused(why.into()))
}

/// [`merge_with`] with the host's own `gh`.
pub fn merge(root: &Path, req: &Request) -> Result<Report, WorktreeError> {
    merge_with(root, req, hq_sandbox::trusted_program("gh").as_deref())
}

pub fn merge_with(root: &Path, req: &Request, gh: Option<&Path>) -> Result<Report, WorktreeError> {
    check_request(req)?;
    let Some(gh) = gh else {
        return Err(WorktreeError::Step(
            "gh_missing",
            "the GitHub CLI (gh) is not installed on the host".into(),
        ));
    };
    let run = Gh { gh, root };
    run.require_login()?;
    let facts = read_facts(&run, req)?;
    judge(&facts).map_err(WorktreeError::Refused)?;
    let mut report = report_of(&facts);
    let Some(approved) = req.approved_sha else {
        return Ok(report);
    };
    if !approved.eq_ignore_ascii_case(&facts.head_sha) {
        return refuse(format!(
            "the pull request moved to {} after the owner approved {}; the owner is asked again",
            short(&facts.head_sha),
            short(approved)
        ));
    }
    run.merge(req, &facts.head_sha)?;
    report.merged = true;
    Ok(report)
}

fn check_request(req: &Request) -> Result<(), WorktreeError> {
    let plain = repo_slug_of_url(req.repo).is_some_and(|s| s.eq_ignore_ascii_case(req.repo));
    if !plain || req.number == 0 {
        return refuse("the pull request must be named as <owner>/<repo> and a number");
    }
    let bad_sha = req.approved_sha.is_some_and(|s| {
        !SHA_LENGTHS.contains(&s.len()) || !s.chars().all(|c| c.is_ascii_hexdigit())
    });
    if bad_sha {
        return refuse("the approved commit is not a full commit id");
    }
    Ok(())
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

fn report_of(f: &PrFacts) -> Report {
    Report {
        repo: f.repo.clone(),
        number: f.number,
        title: f.title.clone(),
        url: f.url.clone(),
        head_sha: f.head_sha.clone(),
        base: f.base.clone(),
        head_ref: f.head_ref.clone(),
        checks: f.passing(),
        merged: false,
    }
}

fn read_facts(run: &Gh, req: &Request) -> Result<PrFacts, WorktreeError> {
    let number = req.number.to_string();
    let default = run.output(&[
        "repo",
        "view",
        req.repo,
        "--json",
        "defaultBranchRef",
        "--jq",
        ".defaultBranchRef.name",
    ])?;
    let view = run.output(&[
        "pr",
        "view",
        &number,
        "--repo",
        req.repo,
        "--json",
        VIEW_FIELDS,
    ])?;
    let parents = run.output(&[
        "api",
        &format!("repos/{}/pulls/{number}/commits", req.repo),
        "--paginate",
        "--jq",
        ".[].parents | length",
    ])?;
    let merge_commits = parents
        .lines()
        .filter(|l| l.trim().parse::<usize>().is_ok_and(|n| n > 1))
        .count();
    facts_from_view(req.repo, &view, default.trim(), merge_commits)
        .map_err(|e| WorktreeError::Step("gh_failed", e))
}

struct Gh<'a> {
    gh: &'a Path,
    root: &'a Path,
}

impl Gh<'_> {
    fn command(&self) -> Command {
        let mut cmd = Command::new(self.gh);
        cmd.current_dir(self.root)
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null());
        cmd
    }

    fn require_login(&self) -> Result<(), WorktreeError> {
        let ok = self
            .command()
            .args(["auth", "status"])
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            return Ok(());
        }
        Err(WorktreeError::Step(
            "gh_not_logged_in",
            "the host's gh is not logged in".into(),
        ))
    }

    fn output(&self, args: &[&str]) -> Result<String, WorktreeError> {
        let out = self
            .command()
            .args(args)
            .output()
            .map_err(|e| WorktreeError::Step("gh_failed", format!("gh could not run: {e}")))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        Err(WorktreeError::Step("gh_failed", last_line(&out.stderr)))
    }

    /// Squash, delete the branch, and only if the head is still the approved commit: GitHub
    /// refuses the merge when a commit landed between the re-read and now.
    fn merge(&self, req: &Request, sha: &str) -> Result<(), WorktreeError> {
        let number = req.number.to_string();
        self.output(&[
            "pr",
            "merge",
            &number,
            "--repo",
            req.repo,
            "--squash",
            "--delete-branch",
            "--match-head-commit",
            sha,
        ])
        .map(|_| ())
    }
}

fn last_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("gh failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba98";

    struct Fixture {
        dir: tempfile::TempDir,
        gh: std::path::PathBuf,
    }

    /// A stub `gh` that answers each subcommand from a file beside it and logs every call.
    fn fixture(view: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::write(d.join("default"), "main\n").unwrap();
        fs::write(d.join("view"), view).unwrap();
        fs::write(d.join("parents"), "1\n1\n").unwrap();
        let gh = d.join("gh");
        let script = format!(
            "#!/bin/sh\nd='{d}'\necho \"$@\" >> \"$d/calls.log\"\n\
             case \"$1 $2\" in\n\
             'auth status') [ -e \"$d/noauth\" ] && exit 1; exit 0;;\n\
             'repo view') cat \"$d/default\";;\n\
             'pr view') cat \"$d/view\";;\n\
             'api repos/'*) cat \"$d/parents\";;\n\
             'pr merge') [ -e \"$d/mergefail\" ] && {{ echo 'head commit changed' >&2; exit 1; }}; exit 0;;\n\
             esac\n",
            d = d.display()
        );
        fs::write(&gh, script).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        Fixture { dir, gh }
    }

    fn view(sha: &str, extra: &str) -> String {
        format!(
            r#"{{"number":7,"title":"Add things","url":"https://example.com/acme/widgets/pull/7",
            "state":"OPEN","isDraft":false,"isCrossRepository":false,"baseRefName":"main",
            "headRefName":"feat/x","headRefOid":"{sha}","mergeable":"MERGEABLE",
            "statusCheckRollup":[{{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}}{extra}]}}"#
        )
    }

    impl Fixture {
        fn run(&self, approved: Option<&str>) -> Result<Report, WorktreeError> {
            let req = Request {
                repo: "acme/widgets",
                number: 7,
                approved_sha: approved,
            };
            merge_with(self.dir.path(), &req, Some(&self.gh))
        }
        fn calls(&self) -> String {
            fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
        }
        fn set(&self, file: &str, text: &str) {
            fs::write(self.dir.path().join(file), text).unwrap();
        }
    }

    fn why(r: Result<Report, WorktreeError>) -> String {
        r.unwrap_err().message().to_string()
    }

    #[test]
    fn a_check_only_read_reports_and_merges_nothing() {
        let f = fixture(&view(SHA, ""));
        let report = f.run(None).unwrap();
        assert!(!report.merged);
        assert_eq!((report.head_sha.as_str(), report.checks.len()), (SHA, 1));
        assert!(!f.calls().contains("pr merge"), "{}", f.calls());
    }

    #[test]
    fn the_approved_commit_merges_once_with_a_squash_pinned_to_it() {
        let f = fixture(&view(SHA, ""));
        let report = f.run(Some(SHA)).unwrap();
        assert!(report.merged);
        let calls = f.calls();
        assert_eq!(calls.matches("pr merge").count(), 1, "{calls}");
        for want in [
            "--squash",
            "--delete-branch",
            &format!("--match-head-commit {SHA}"),
        ] {
            assert!(calls.contains(want), "{want} in {calls}");
        }
        assert!(
            !calls.contains("--admin") && !calls.contains("--auto"),
            "{calls}"
        );
    }

    #[test]
    fn a_commit_pushed_after_the_approval_voids_it() {
        let f = fixture(&view(OTHER, ""));
        let msg = why(f.run(Some(SHA)));
        assert!(
            msg.contains("moved") && msg.contains("asked again"),
            "{msg}"
        );
        assert!(!f.calls().contains("pr merge"));
    }

    #[test]
    fn a_check_gone_red_after_the_approval_stops_the_merge() {
        let red = r#",{"name":"lint","status":"COMPLETED","conclusion":"FAILURE"}"#;
        let f = fixture(&view(SHA, red));
        assert!(why(f.run(Some(SHA))).contains("checks failed: lint"));
        assert!(!f.calls().contains("pr merge"));
    }

    #[test]
    fn a_merge_commit_or_a_wrong_base_is_refused() {
        let f = fixture(&view(SHA, ""));
        f.set("parents", "1\n2\n");
        assert!(why(f.run(None)).contains("merge commit"));
        f.set("parents", "1\n");
        f.set("default", "trunk\n");
        assert!(why(f.run(None)).contains("default branch trunk"));
    }

    #[test]
    fn a_failed_merge_is_reported_and_a_missing_login_or_gh_is_named() {
        let f = fixture(&view(SHA, ""));
        f.set("mergefail", "");
        assert!(why(f.run(Some(SHA))).contains("head commit changed"));
        f.set("noauth", "");
        assert_eq!(f.run(None).unwrap_err().code(), "gh_not_logged_in");
        let req = Request {
            repo: "acme/widgets",
            number: 7,
            approved_sha: None,
        };
        let missing = merge_with(f.dir.path(), &req, None).unwrap_err();
        assert_eq!(missing.code(), "gh_missing");
    }

    #[test]
    fn malformed_requests_never_reach_gh() {
        let f = fixture(&view(SHA, ""));
        for (repo, number, sha) in [
            ("acme", 7, None),
            ("acme/widgets", 0, None),
            ("acme/widgets", 7, Some("main")),
            ("acme/widgets", 7, Some("abc")),
        ] {
            let req = Request {
                repo,
                number,
                approved_sha: sha,
            };
            assert!(merge_with(f.dir.path(), &req, Some(&f.gh)).is_err());
        }
        assert!(f.calls().is_empty());
    }
}
