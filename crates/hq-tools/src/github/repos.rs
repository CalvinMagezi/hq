//! What HQ knows about each repository the App can reach. A pull request from an `hq/` branch
//! runs CI with the repository's secrets, and preview hosts build it with theirs, before anyone
//! reviews it, so a repository that has either waits until the owner confirms it.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::app::{AppClient, AppToken, Step, TokenRefused};

pub const SCAN_FILE: &str = "repos.json";
const ACCEPT_JSON: &str = "application/vnd.github+json";
const ACCEPT_RAW: &str = "application/vnd.github.raw+json";
const MAX_REPOS: usize = 30;
const MAX_WORKFLOWS: usize = 30;
const REPOS_PER_PAGE: usize = 100;
const OWN_TOKEN: &str = "GITHUB_TOKEN";
const ALL_SECRETS: &str = "every secret";
const WORKFLOWS_DIR: &str = ".github/workflows";
const PREVIEW_FILES: [&str; 6] = [
    "vercel.json",
    ".vercel",
    "netlify.toml",
    "wrangler.toml",
    "wrangler.json",
    "wrangler.jsonc",
];

// GitHub's expressions ignore case.
static SECRET_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bsecrets\s*(?:\.\s*([A-Za-z_][A-Za-z0-9_]*)|\[\s*['"]([^'"]+)['"]\s*\])"#)
        .unwrap()
});
/// A computed index such as `secrets[matrix.name]` can reach any secret.
static EVERY_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\bsecrets\s*:\s*inherit\b|\btoJSON\(\s*secrets\s*\)|\bsecrets\s*\[\s*[^'"\s]"#,
    )
    .unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoReport {
    pub repo: String,
    pub private: bool,
    pub default_branch: String,
    /// Whether GitHub itself protects the default branch.
    pub protected: bool,
    pub has_ci: bool,
    /// Secrets the workflows use, beyond the `GITHUB_TOKEN` every run has.
    pub secrets: Vec<String>,
    pub previews: Vec<String>,
    /// HQ could not read it all, so it cannot say the repository is safe.
    pub unchecked: bool,
    #[serde(default)]
    pub confirmed: bool,
}

impl RepoReport {
    pub fn flagged(&self) -> bool {
        self.unchecked || !self.secrets.is_empty() || !self.previews.is_empty()
    }

    pub fn picked(&self) -> bool {
        !self.flagged() || self.confirmed
    }
}

/// The secrets one workflow file refers to, leaving out `GITHUB_TOKEN`.
pub fn secret_names(workflow: &str) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = SECRET_REF
        .captures_iter(workflow)
        .filter_map(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| m.as_str().to_string())
        .filter(|name| !name.eq_ignore_ascii_case(OWN_TOKEN))
        .collect();
    if EVERY_SECRET.is_match(workflow) {
        names.insert(ALL_SECRETS.to_string());
    }
    names
}

/// One repository's API, with a token for it.
struct Reader<'a> {
    app: &'a AppClient,
    repo: &'a str,
    token: &'a AppToken,
}

impl Reader<'_> {
    async fn get(&self, path: &str, accept: &str) -> Result<Option<reqwest::Response>> {
        let (client, base) = self.app.http();
        let url = match path {
            "" => format!("{base}repos/{}", self.repo),
            _ => format!("{base}repos/{}/{path}", self.repo),
        };
        let response = client
            .get(url)
            .header("Accept", accept)
            .bearer_auth(self.token.secret())
            .send()
            .await?;
        match response.status().as_u16() {
            404 => Ok(None),
            200 => Ok(Some(response)),
            other => anyhow::bail!("GitHub answered {other} for {path}"),
        }
    }

    async fn json(&self, path: &str) -> Result<Option<Value>> {
        match self.get(path, ACCEPT_JSON).await? {
            Some(response) => Ok(Some(response.json().await?)),
            None => Ok(None),
        }
    }

    async fn text(&self, path: &str) -> Result<Option<String>> {
        match self.get(path, ACCEPT_RAW).await? {
            Some(response) => Ok(Some(response.text().await?)),
            None => Ok(None),
        }
    }

    async fn workflow_secrets(&self, report: &mut RepoReport) -> Result<()> {
        let Some(listed) = self.json(&format!("contents/{WORKFLOWS_DIR}")).await? else {
            return Ok(());
        };
        let files: Vec<&str> = listed
            .as_array()
            .context("the workflows folder was not a list")?
            .iter()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .filter(|name| name.ends_with(".yml") || name.ends_with(".yaml"))
            .collect();
        report.has_ci = !files.is_empty();
        report.unchecked |= files.len() > MAX_WORKFLOWS;
        let mut found = BTreeSet::new();
        for name in files.iter().take(MAX_WORKFLOWS) {
            let text = self
                .text(&format!("contents/{WORKFLOWS_DIR}/{name}"))
                .await?;
            found.extend(secret_names(&text.unwrap_or_default()));
        }
        report.secrets = found.into_iter().collect();
        Ok(())
    }

    async fn preview_files(&self, report: &mut RepoReport) -> Result<()> {
        let listed = self.json("contents/").await?;
        let names = listed
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        report.previews = names
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .filter(|name| PREVIEW_FILES.contains(name))
            .map(str::to_string)
            .collect();
        Ok(())
    }

    async fn protection(&self, report: &mut RepoReport) -> Result<()> {
        let branch = self
            .json(&format!("branches/{}", report.default_branch))
            .await?;
        report.protected = branch
            .and_then(|b| b.get("protected").and_then(Value::as_bool))
            .unwrap_or(false);
        Ok(())
    }
}

fn blank_report(info: &Value) -> Option<RepoReport> {
    Some(RepoReport {
        repo: info.get("full_name")?.as_str()?.to_string(),
        private: info.get("private").and_then(Value::as_bool).unwrap_or(true),
        default_branch: info.get("default_branch")?.as_str()?.to_string(),
        protected: false,
        has_ci: false,
        secrets: Vec::new(),
        previews: Vec::new(),
        unchecked: false,
        confirmed: false,
    })
}

impl AppClient {
    pub fn repositories(&self) -> Vec<RepoReport> {
        self.store()
            .read_file(SCAN_FILE)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_repositories(&self, reports: &[RepoReport]) -> Result<()> {
        self.store()
            .write_private(SCAN_FILE, &serde_json::to_vec_pretty(reports)?)
    }

    async fn list_installation(&self) -> Result<Vec<Value>> {
        let token = self.listing_token().await?;
        let (client, base) = self.http();
        let response = client
            .get(format!(
                "{base}installation/repositories?per_page={REPOS_PER_PAGE}"
            ))
            .header("Accept", ACCEPT_JSON)
            .bearer_auth(token.secret())
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "GitHub would not list the repositories ({})",
                response.status()
            );
        }
        let value: Value = response.json().await?;
        Ok(value
            .get("repositories")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// A failure to read marks the repository unchecked rather than safe.
    async fn inspect(&self, mut report: RepoReport) -> RepoReport {
        let checked = self.read_into(&mut report).await;
        report.unchecked |= checked.is_err();
        report
    }

    async fn read_into(&self, report: &mut RepoReport) -> Result<()> {
        let repo = report.repo.clone();
        let token = self.token(&repo, Step::Fetch).await?;
        let reader = Reader {
            app: self,
            repo: &repo,
            token: &token,
        };
        reader.workflow_secrets(report).await?;
        reader.preview_files(report).await?;
        reader.protection(report).await
    }

    /// Reads every repository the person picked, keeping confirmations for ones unchanged.
    pub async fn scan_repositories(&self) -> Result<Vec<RepoReport>> {
        let before = self.repositories();
        let listed = self.list_installation().await?;
        let mut reports = Vec::new();
        for info in listed.iter().take(MAX_REPOS) {
            let Some(blank) = blank_report(info) else {
                continue;
            };
            let mut report = self.inspect(blank).await;
            report.confirmed = confirmation_holds(&before, &report);
            reports.push(report);
        }
        self.save_repositories(&reports)?;
        Ok(reports)
    }

    /// Owner only. False when HQ has no report for that repository.
    pub fn confirm_repository(&self, repo: &str) -> Result<bool> {
        let mut reports = self.repositories();
        let Some(found) = reports
            .iter_mut()
            .find(|r| r.repo.eq_ignore_ascii_case(repo))
        else {
            return Ok(false);
        };
        found.confirmed = true;
        self.save_repositories(&reports)?;
        Ok(true)
    }

    /// Refuses work on a flagged repository the owner has not confirmed. Read afresh every time,
    /// so a workflow that gained a secret since the last scan, or a repository added since, counts.
    pub async fn ensure_picked(&self, repo: &str) -> Result<()> {
        let mut report = self.first_look(repo).await?;
        let mut reports = self.repositories();
        report.confirmed = confirmation_holds(&reports, &report);
        reports.retain(|r| !r.repo.eq_ignore_ascii_case(repo));
        reports.push(report.clone());
        self.save_repositories(&reports)?;
        if !report.picked() {
            return Err(TokenRefused::NeedsConfirmation(repo.to_string()).into());
        }
        Ok(())
    }

    async fn first_look(&self, repo: &str) -> Result<RepoReport> {
        let token = self.token(repo, Step::Fetch).await?;
        let reader = Reader {
            app: self,
            repo,
            token: &token,
        };
        let info = reader
            .json("")
            .await?
            .with_context(|| format!("GitHub has no {repo}"))?;
        let blank = blank_report(&info)
            .with_context(|| format!("GitHub's answer for {repo} was incomplete"))?;
        Ok(self.inspect(blank).await)
    }
}

/// A confirmation covers what the owner saw; a new secret or preview asks again.
fn confirmation_holds(before: &[RepoReport], now: &RepoReport) -> bool {
    // A failed read shows no secrets, which says nothing about whether there are any.
    !now.unchecked
        && before.iter().any(|old| {
            old.confirmed
                && old.repo == now.repo
                && now.secrets.iter().all(|s| old.secrets.contains(s))
                && now.previews.iter().all(|p| old.previews.contains(p))
        })
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
