//! What must be true of a pull request before HQ asks the owner to merge it, and again right
//! before HQ merges it. Pure functions over what `gh` reported, so the rules live in one place
//! and run the same in the tool, the host and the tests.

use crate::publish_check::{is_protected_branch, repo_slug_of_url};
use serde::Deserialize;

/// Why a pull request is never merged, for the config's `never_merge` list.
pub const NEVER_MERGE_REASON: &str =
    "this repository's rule is to open pull requests and never merge them";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Passed,
    Pending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
}

/// Everything the rules read, gathered by the host from `gh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFacts {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub cross_repository: bool,
    pub base: String,
    pub head_ref: String,
    pub head_sha: String,
    pub mergeable: String,
    pub default_branch: String,
    pub checks: Vec<Check>,
    pub merge_commits: usize,
}

impl PrFacts {
    /// Names of the checks that passed, for the owner's approval text.
    pub fn passing(&self) -> Vec<String> {
        self.checks.iter().map(|c| c.name.clone()).collect()
    }
}

/// The first rule a pull request breaks, worded for the helper that asked.
pub fn judge(f: &PrFacts) -> Result<(), String> {
    if f.state != "OPEN" {
        return Err(format!(
            "the pull request is {}, not open",
            f.state.to_lowercase()
        ));
    }
    if f.is_draft {
        return Err("the pull request is a draft; mark it ready for review first".into());
    }
    if f.cross_repository {
        return Err(
            "the pull request comes from a fork; HQ merges only branches of the repository itself"
                .into(),
        );
    }
    if f.base != f.default_branch {
        return Err(format!(
            "the pull request targets {}, not the default branch {}",
            f.base, f.default_branch
        ));
    }
    if is_protected_branch(&f.head_ref, Some(&f.default_branch)) {
        return Err(format!(
            "{} is a protected branch and is never merged from",
            f.head_ref
        ));
    }
    if f.mergeable != "MERGEABLE" {
        return Err(format!(
            "the pull request is not mergeable ({}); resolve conflicts first",
            f.mergeable.to_lowercase()
        ));
    }
    if f.merge_commits > 0 {
        return Err("the branch holds a merge commit; rebase it onto the base first".into());
    }
    judge_checks(&f.checks)
}

fn judge_checks(checks: &[Check]) -> Result<(), String> {
    if checks.is_empty() {
        return Err("no checks reported, so HQ cannot tell the pull request is green".into());
    }
    let named = |state| {
        checks
            .iter()
            .filter(|c| c.state == state)
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let failed = named(CheckState::Failed);
    if !failed.is_empty() {
        return Err(format!("checks failed: {failed}"));
    }
    let pending = named(CheckState::Pending);
    if !pending.is_empty() {
        return Err(format!("checks still running: {pending}"));
    }
    Ok(())
}

/// A pull request named as `<owner>/<repo>#<n>` or by its address
/// (`https://host/<owner>/<repo>/pull/<n>`): the repository and the number.
pub fn parse_pr_ref(text: &str) -> Option<(String, u64)> {
    let text = text.trim();
    if text.starts_with("https://") {
        return parse_pr_url(text);
    }
    if let Some((repo, n)) = text.rsplit_once('#') {
        let number = n.parse::<u64>().ok().filter(|n| *n > 0)?;
        let slug = repo_slug_of_url(repo).filter(|s| s.eq_ignore_ascii_case(repo))?;
        return Some((slug, number));
    }
    None
}

fn parse_pr_url(text: &str) -> Option<(String, u64)> {
    let url = text.split(['?', '#']).next()?.trim_end_matches('/');
    let rest = url.strip_prefix("https://")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let [_host, owner, name, "pull", n] = parts[..] else {
        return None;
    };
    let number = n.parse::<u64>().ok().filter(|n| *n > 0)?;
    let slug = repo_slug_of_url(&format!("{owner}/{name}"))?;
    Some((slug, number))
}

#[derive(Deserialize)]
struct RawView {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(default, rename = "isCrossRepository")]
    cross_repository: bool,
    #[serde(rename = "baseRefName")]
    base: String,
    #[serde(rename = "headRefName")]
    head_ref: String,
    #[serde(rename = "headRefOid")]
    head_sha: String,
    #[serde(default)]
    mergeable: String,
    #[serde(default, rename = "statusCheckRollup")]
    rollup: Vec<RawCheck>,
}

/// A check run (`status`, `conclusion`) or a commit status (`state`) in `statusCheckRollup`.
#[derive(Deserialize)]
struct RawCheck {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

impl RawCheck {
    fn into_check(self) -> Check {
        let name = self
            .name
            .or(self.context)
            .unwrap_or_else(|| "unnamed check".into());
        let verdict = self
            .conclusion
            .or(self.state)
            .unwrap_or_default()
            .to_ascii_uppercase();
        let running = self
            .status
            .is_some_and(|s| !s.eq_ignore_ascii_case("COMPLETED"));
        let state = match verdict.as_str() {
            _ if running => CheckState::Pending,
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckState::Passed,
            "" | "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" => CheckState::Pending,
            _ => CheckState::Failed,
        };
        Check { name, state }
    }
}

/// `gh pr view --json ...` plus what the other two `gh` calls found, as facts for [`judge`].
pub fn facts_from_view(
    repo: &str,
    view_json: &str,
    default_branch: &str,
    merge_commits: usize,
) -> Result<PrFacts, String> {
    let raw: RawView = serde_json::from_str(view_json)
        .map_err(|e| format!("could not read the pull request from gh: {e}"))?;
    Ok(PrFacts {
        repo: repo.to_string(),
        number: raw.number,
        title: raw.title,
        url: raw.url,
        state: raw.state.to_ascii_uppercase(),
        is_draft: raw.is_draft,
        cross_repository: raw.cross_repository,
        base: raw.base,
        head_ref: raw.head_ref,
        head_sha: raw.head_sha,
        mergeable: raw.mergeable.to_ascii_uppercase(),
        default_branch: default_branch.to_string(),
        checks: raw.rollup.into_iter().map(RawCheck::into_check).collect(),
        merge_commits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn green() -> PrFacts {
        PrFacts {
            repo: "acme/widgets".into(),
            number: 7,
            title: "Add things".into(),
            url: "https://example.com/acme/widgets/pull/7".into(),
            state: "OPEN".into(),
            is_draft: false,
            cross_repository: false,
            base: "main".into(),
            head_ref: "feat/x".into(),
            head_sha: "abc123".into(),
            mergeable: "MERGEABLE".into(),
            default_branch: "main".into(),
            checks: vec![Check {
                name: "ci".into(),
                state: CheckState::Passed,
            }],
            merge_commits: 0,
        }
    }

    #[test]
    fn a_green_pull_request_passes() {
        assert!(judge(&green()).is_ok());
    }

    #[test]
    fn each_broken_rule_is_named() {
        type Case = (Box<dyn Fn(&mut PrFacts)>, &'static str);
        let cases: Vec<Case> = vec![
            (Box::new(|f| f.state = "MERGED".into()), "not open"),
            (Box::new(|f| f.is_draft = true), "draft"),
            (Box::new(|f| f.cross_repository = true), "fork"),
            (Box::new(|f| f.base = "develop".into()), "default branch"),
            (Box::new(|f| f.head_ref = "main".into()), "protected"),
            (Box::new(|f| f.head_ref = "release/1".into()), "protected"),
            (
                Box::new(|f| f.mergeable = "CONFLICTING".into()),
                "not mergeable",
            ),
            (Box::new(|f| f.merge_commits = 1), "merge commit"),
            (Box::new(|f| f.checks.clear()), "no checks"),
            (
                Box::new(|f| {
                    f.checks.push(Check {
                        name: "lint".into(),
                        state: CheckState::Failed,
                    })
                }),
                "checks failed: lint",
            ),
            (
                Box::new(|f| {
                    f.checks.push(Check {
                        name: "e2e".into(),
                        state: CheckState::Pending,
                    })
                }),
                "still running: e2e",
            ),
        ];
        for (mutate, want) in cases {
            let mut f = green();
            mutate(&mut f);
            let why = judge(&f).unwrap_err();
            assert!(why.contains(want), "{want}: {why}");
        }
    }

    #[test]
    fn pull_requests_are_named_by_slug_or_address() {
        let want = Some(("acme/widgets".to_string(), 7));
        assert_eq!(parse_pr_ref("acme/widgets#7"), want);
        assert_eq!(
            parse_pr_ref("https://example.com/acme/widgets/pull/7"),
            want
        );
        assert_eq!(
            parse_pr_ref("https://example.com/acme/widgets/pull/7/files?x=1"),
            None
        );
        assert_eq!(
            parse_pr_ref("https://example.com/acme/widgets/pull/7/"),
            want
        );
        for bad in [
            "",
            "acme/widgets",
            "acme/widgets#0",
            "acme#7",
            "x y/z#1",
            "http://example.com/a/b/pull/1",
            "https://example.com/a/b/issues/1",
        ] {
            assert_eq!(parse_pr_ref(bad), None, "{bad}");
        }
    }

    #[test]
    fn gh_json_becomes_facts() {
        let json = r#"{"number":7,"title":"T","url":"u","state":"OPEN","isDraft":false,
          "isCrossRepository":false,"baseRefName":"main","headRefName":"feat/x",
          "headRefOid":"abc","mergeable":"MERGEABLE","statusCheckRollup":[
          {"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"},
          {"name":"slow","status":"IN_PROGRESS","conclusion":""},
          {"context":"legacy","state":"FAILURE"},
          {"name":"opt","status":"COMPLETED","conclusion":"SKIPPED"}]}"#;
        let f = facts_from_view("acme/widgets", json, "main", 0).unwrap();
        let states: Vec<_> = f.checks.iter().map(|c| c.state).collect();
        assert_eq!(
            states,
            [
                CheckState::Passed,
                CheckState::Pending,
                CheckState::Failed,
                CheckState::Passed
            ]
        );
        assert!(facts_from_view("a/b", "not json", "main", 0).is_err());
    }
}
