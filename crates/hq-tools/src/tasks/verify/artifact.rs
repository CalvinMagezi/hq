//! Finding what a task produced, so there is something to verify.
//!
//! The task's own links come first, because an agent that linked its pull request has said which
//! one it means. When none is linked, the lease records the branch the work was done on, and a
//! pull request is found from that. A task with nothing discoverable has no artifact, and cannot
//! reach a machine level of verification.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use hq_db::tasks as t;
use rusqlite::Connection;
use serde_json::Value;

use super::collect::encode_query_value;
use super::remote::GitHubRead;

/// Branches that are where work lands, not where one task's work was done.
const SHARED_BRANCHES: [&str; 7] = [
    "main",
    "master",
    "develop",
    "dev",
    "trunk",
    "staging",
    "production",
];
/// Largest git config read to find a remote.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Pull requests taken from one branch lookup.
const PRS_PER_BRANCH: usize = 3;

/// Something a task produced that can be checked.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Artifact {
    Pr { repo: String, number: u64 },
    Commit { repo: Option<String>, sha: String },
    Note { path: String },
}

impl Artifact {
    /// The form stored on a verdict: `pr:owner/repo#7`, `commit:owner/repo@sha`, `note:path`.
    pub fn label(&self) -> String {
        match self {
            Artifact::Pr { repo, number } => format!("pr:{repo}#{number}"),
            Artifact::Commit { repo: Some(r), sha } => format!("commit:{r}@{sha}"),
            Artifact::Commit { repo: None, sha } => format!("commit:{sha}"),
            Artifact::Note { path } => format!("note:{path}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFact {
    pub kind: String,
    pub reference: String,
    pub direction: String,
}

/// The branch a lease worked on and where that session ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseBranch {
    pub branch: String,
    pub cwd: String,
}

/// What the database knows about a task that discovery needs.
#[derive(Debug, Clone, Default)]
pub struct TaskFacts {
    pub links: Vec<LinkFact>,
    pub branches: Vec<LeaseBranch>,
    /// Notes recorded as written under the task.
    pub produced_notes: Vec<String>,
}

impl TaskFacts {
    pub fn load(conn: &Connection, task_id: &str) -> Result<Self> {
        let links = t::list_task_links(conn, task_id)?
            .into_iter()
            .map(|l| LinkFact {
                kind: l.kind,
                reference: l.reference,
                direction: l.direction,
            })
            .collect();
        let mut seen = BTreeSet::new();
        let branches = t::list_work_sessions(conn, task_id, 30)?
            .into_iter()
            .filter(|s| !s.branch.is_empty())
            .filter(|s| seen.insert((s.branch.clone(), s.cwd.clone())))
            .map(|s| LeaseBranch {
                branch: s.branch,
                cwd: s.cwd,
            })
            .collect();
        let produced_notes = hq_db::provenance::for_task(conn, task_id, 50)?
            .into_iter()
            .map(|p| p.path)
            .collect();
        Ok(Self {
            links,
            branches,
            produced_notes,
        })
    }
}

/// What discovery found, and what it could not resolve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub artifacts: Vec<Artifact>,
    /// How anything that was not a link was found, and what could not be looked up.
    pub notes: Vec<String>,
}

impl Discovery {
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub fn prs(&self) -> impl Iterator<Item = (&str, u64)> {
        self.artifacts.iter().filter_map(|a| match a {
            Artifact::Pr { repo, number } => Some((repo.as_str(), *number)),
            _ => None,
        })
    }
}

/// `owner/repo#7`, as links store a pull request.
fn parse_pr(reference: &str) -> Option<(String, u64)> {
    let (repo, number) = reference.rsplit_once('#')?;
    Some((repo.to_ascii_lowercase(), number.parse().ok()?))
}

/// `sha` or `owner/repo@sha`.
fn parse_commit(reference: &str) -> Option<(Option<String>, String)> {
    let (repo, sha) = match reference.split_once('@') {
        Some((r, s)) => (Some(r.to_ascii_lowercase()), s),
        None => (None, reference),
    };
    (sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| (repo, sha.to_ascii_lowercase()))
}

/// Finds a task's artifacts: linked pull requests, commits and produced notes first, then pull
/// requests whose head is a branch a lease worked on. `cwd_repos` maps a session's working
/// directory to the repository checked out there.
pub async fn discover(
    gh: &dyn GitHubRead,
    facts: &TaskFacts,
    cwd_repos: &HashMap<String, String>,
) -> Discovery {
    let mut out = Discovery::default();
    let mut seen = BTreeSet::new();
    let mut add = |out: &mut Discovery, a: Artifact| {
        if seen.insert(a.clone()) {
            out.artifacts.push(a);
        }
    };
    let mut repos: BTreeSet<String> = BTreeSet::new();

    for link in &facts.links {
        match link.kind.as_str() {
            "pr" => {
                if let Some((repo, number)) = parse_pr(&link.reference) {
                    repos.insert(repo.clone());
                    add(&mut out, Artifact::Pr { repo, number });
                }
            }
            "commit" => {
                if let Some((repo, sha)) = parse_commit(&link.reference) {
                    if let Some(r) = &repo {
                        repos.insert(r.clone());
                    }
                    add(&mut out, Artifact::Commit { repo, sha });
                }
            }
            // A note the task came from is its source, not its product.
            "vault_note" if link.direction == "produced" => {
                add(
                    &mut out,
                    Artifact::Note {
                        path: link.reference.clone(),
                    },
                );
            }
            _ => {}
        }
    }
    for path in &facts.produced_notes {
        add(&mut out, Artifact::Note { path: path.clone() });
    }

    // No pull request linked: look for one from the branches the work was done on.
    let has_pr = out
        .artifacts
        .iter()
        .any(|a| matches!(a, Artifact::Pr { .. }));
    if has_pr {
        return out;
    }
    for lease in &facts.branches {
        if SHARED_BRANCHES.contains(&lease.branch.to_ascii_lowercase().as_str()) {
            continue;
        }
        let mut candidates = repos.clone();
        if let Some(r) = cwd_repos.get(&lease.cwd) {
            candidates.insert(r.clone());
        }
        if candidates.is_empty() {
            out.notes.push(format!(
                "branch {} was worked on, but no repository is known for it",
                lease.branch
            ));
            continue;
        }
        for repo in candidates {
            let owner = repo.split('/').next().unwrap_or_default();
            let path = format!(
                "repos/{repo}/pulls?head={owner}:{}&state=all&per_page={PRS_PER_BRANCH}",
                encode_query_value(&lease.branch)
            );
            match gh.api_json(&path).await {
                Ok(Value::Array(prs)) if !prs.is_empty() => {
                    for pr in prs.iter().take(PRS_PER_BRANCH) {
                        if let Some(number) = pr.get("number").and_then(Value::as_u64) {
                            add(
                                &mut out,
                                Artifact::Pr {
                                    repo: repo.clone(),
                                    number,
                                },
                            );
                            out.notes.push(format!(
                                "{repo}#{number} was found from the lease branch {}",
                                lease.branch
                            ));
                        }
                    }
                }
                Ok(_) => out.notes.push(format!(
                    "no pull request in {repo} has the head branch {}",
                    lease.branch
                )),
                Err(e) => out.notes.push(format!(
                    "pull requests for {} in {repo} could not be looked up ({e})",
                    lease.branch
                )),
            }
        }
    }
    out
}

/// `owner/repo` from a git remote URL, lower-cased, when it is a GitHub one.
pub fn parse_github_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let after_scheme = url.split_once("://").map(|(_, r)| r)?;
        let no_creds = after_scheme
            .rsplit_once('@')
            .map_or(after_scheme, |(_, r)| r);
        no_creds
            .strip_prefix("github.com/")
            .or_else(|| no_creds.strip_prefix("github.com:"))?
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || owner.is_empty() || name.is_empty() {
        return None;
    }
    let repo = format!("{owner}/{name}").to_ascii_lowercase();
    crate::github::is_valid_repo(&repo).then_some(repo)
}

fn read_capped(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_CONFIG_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// The config file of the repository checked out at `cwd`, following a worktree's `.git` file to
/// the repository it belongs to. Reads files only; it never runs git against a directory an
/// agent named.
fn git_config_path(cwd: &Path) -> Option<PathBuf> {
    let dot_git = cwd.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git.join("config"));
    }
    let pointer = read_capped(&dot_git)?;
    let gitdir = PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
    let common = read_capped(&gitdir.join("commondir"))
        .map(|c| gitdir.join(c.trim()))
        .unwrap_or(gitdir);
    Some(common.join("config"))
}

/// The GitHub repository the checkout at `cwd` pushes to, from its `origin` remote.
pub fn repo_of_checkout(cwd: &str) -> Option<String> {
    let cwd = Path::new(cwd);
    if !cwd.is_absolute() {
        return None;
    }
    let config = read_capped(&git_config_path(cwd)?)?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line
                .replace(' ', "")
                .eq_ignore_ascii_case("[remote\"origin\"]");
        } else if in_origin
            && let Some(url) = line.strip_prefix("url")
            && let Some(url) = url.trim_start().strip_prefix('=')
        {
            return parse_github_remote(url);
        }
    }
    None
}

/// The repository for each working directory a task's leases ran in, where one can be read.
pub fn repos_for_cwds<'a>(cwds: impl IntoIterator<Item = &'a str>) -> HashMap<String, String> {
    cwds.into_iter()
        .filter_map(|cwd| repo_of_checkout(cwd).map(|r| (cwd.to_string(), r)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::remote::fake::FakeGitHub;
    use super::*;
    use serde_json::json;

    fn link(kind: &str, reference: &str, direction: &str) -> LinkFact {
        LinkFact {
            kind: kind.into(),
            reference: reference.into(),
            direction: direction.into(),
        }
    }

    #[tokio::test]
    async fn linked_work_is_found_and_the_note_a_task_came_from_is_not_its_product() {
        let facts = TaskFacts {
            links: vec![
                link("pr", "Example/Repo#7", "produced"),
                link("commit", "example/repo@ABCDEF1", "produced"),
                link("commit", "1234567", "related"),
                link("vault_note", "Notebooks/plan.md", "origin"),
                link("vault_note", "Notebooks/result.md", "produced"),
                link("url", "https://example.com", "related"),
            ],
            produced_notes: vec!["Notebooks/written.md".into(), "Notebooks/result.md".into()],
            ..Default::default()
        };
        let d = discover(&FakeGitHub::default(), &facts, &HashMap::new()).await;
        let labels: Vec<String> = d.artifacts.iter().map(Artifact::label).collect();
        assert_eq!(
            labels,
            [
                "pr:example/repo#7",
                "commit:example/repo@abcdef1",
                "commit:1234567",
                "note:Notebooks/result.md",
                "note:Notebooks/written.md",
            ]
        );
    }

    #[tokio::test]
    async fn a_pr_is_found_from_the_lease_branch_when_none_is_linked() {
        let gh = FakeGitHub::default().with(
            "repos/example/repo/pulls?head=example:claude/fix-x&state=all&per_page=3",
            json!([{"number": 41}, {"number": 40}]),
        );
        let facts = TaskFacts {
            branches: vec![LeaseBranch {
                branch: "claude/fix-x".into(),
                cwd: "/work/repo".into(),
            }],
            ..Default::default()
        };
        let cwd_repos = HashMap::from([("/work/repo".to_string(), "example/repo".to_string())]);
        let d = discover(&gh, &facts, &cwd_repos).await;
        assert_eq!(
            d.prs().collect::<Vec<_>>(),
            [("example/repo", 41), ("example/repo", 40)]
        );
        assert!(
            d.notes
                .iter()
                .any(|n| n.contains("found from the lease branch"))
        );
    }

    #[tokio::test]
    async fn a_linked_pr_is_enough_and_the_branch_is_not_searched() {
        let gh = FakeGitHub::default();
        let facts = TaskFacts {
            links: vec![link("pr", "example/repo#7", "produced")],
            branches: vec![LeaseBranch {
                branch: "feature".into(),
                cwd: "/w".into(),
            }],
            ..Default::default()
        };
        let d = discover(&gh, &facts, &HashMap::new()).await;
        assert_eq!(d.artifacts.len(), 1);
        assert!(gh.asked.lock().unwrap().is_empty(), "no lookup was made");
    }

    #[tokio::test]
    async fn shared_branches_unknown_repos_and_failed_lookups_are_said_not_guessed() {
        let facts = TaskFacts {
            branches: vec![
                LeaseBranch {
                    branch: "main".into(),
                    cwd: "/w".into(),
                },
                LeaseBranch {
                    branch: "feature".into(),
                    cwd: "/elsewhere".into(),
                },
            ],
            ..Default::default()
        };
        let d = discover(&FakeGitHub::default(), &facts, &HashMap::new()).await;
        assert!(d.is_empty());
        assert_eq!(
            d.notes.len(),
            1,
            "main is skipped; feature has no known repository"
        );
        assert!(d.notes[0].contains("no repository is known"));

        let cwd_repos = HashMap::from([("/elsewhere".to_string(), "example/repo".to_string())]);
        let d = discover(&FakeGitHub::default(), &facts, &cwd_repos).await;
        assert!(d.is_empty());
        assert!(
            d.notes[0].contains("could not be looked up"),
            "{:?}",
            d.notes
        );

        let gh = FakeGitHub::default().with(
            "repos/example/repo/pulls?head=example:feature&state=all&per_page=3",
            json!([]),
        );
        let d = discover(&gh, &facts, &cwd_repos).await;
        assert!(d.notes[0].contains("no pull request"), "{:?}", d.notes);
    }

    #[test]
    fn github_remotes_in_every_common_form_give_the_repository() {
        for url in [
            "git@github.com:Example/Repo.git",
            "https://github.com/Example/Repo",
            "https://github.com/Example/Repo.git",
            "https://user:token@github.com/Example/Repo.git",
            "ssh://git@github.com/Example/Repo.git",
            "https://github.com/Example/Repo/",
        ] {
            assert_eq!(
                parse_github_remote(url).as_deref(),
                Some("example/repo"),
                "{url}"
            );
        }
        for url in [
            "https://gitlab.com/example/repo.git",
            "git@bitbucket.org:example/repo.git",
            "https://github.com/example",
            "https://github.com/a/b/c",
            "/local/path/repo",
            "",
        ] {
            assert_eq!(parse_github_remote(url), None, "{url}");
        }
    }

    #[test]
    fn a_checkouts_repository_comes_from_its_origin_remote() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git/config"),
            "[core]\n\turl = https://github.com/wrong/place\n[remote \"upstream\"]\n\turl = https://github.com/up/stream\n[remote \"origin\"]\n\turl = git@github.com:Example/Repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        )
        .unwrap();
        assert_eq!(
            repo_of_checkout(dir.path().to_str().unwrap()).as_deref(),
            Some("example/repo")
        );
        assert_eq!(
            repo_of_checkout("relative/path"),
            None,
            "only absolute paths are read"
        );
        assert_eq!(repo_of_checkout("/definitely/not/here"), None);
    }

    #[test]
    fn a_worktree_finds_the_repository_it_belongs_to() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        let wt = root.path().join("wt");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://github.com/Example/Repo.git\n",
        )
        .unwrap();
        std::fs::write(main.join(".git/worktrees/wt/commondir"), "../..\n").unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        assert_eq!(
            repo_of_checkout(wt.to_str().unwrap()).as_deref(),
            Some("example/repo")
        );
        let map = repos_for_cwds([wt.to_str().unwrap(), "/nope"]);
        assert_eq!(map.len(), 1);
    }
}
