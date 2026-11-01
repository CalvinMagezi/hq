//! `github_write`: a tarball checkout's changes as an `hq/` branch and a pull request, nothing more.
//! A worker reads untrusted repository text by design, so instead of a taint gate everything
//! published is scanned for credentials (see `docs/plans/edge-coding-worker.md`).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use base64::Engine;
use regex::Regex;
use reqwest::Method;
use serde_json::{Value, json};

use super::changes::{Change, changes};
use super::tarball::{Base, load_base};
use super::{API_HOST_URL, InFlight, USER_AGENT, clone_root, records_root, repo, text};
use crate::registry::HqTool;

/// The token's own variable, never the read-only `GH_TOKEN` or `GITHUB_TOKEN`.
pub(crate) const TOKEN_ENV: &str = "HQ_GITHUB_WRITE_TOKEN";
const BRANCH_PREFIX: &str = "hq/";
const MAX_BRANCH_CHARS: usize = 100;
const MAX_MESSAGE_CHARS: usize = 10_000;
/// GitHub's own limits, checked before anything is pushed so a refusal never comes halfway.
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_CHARS: usize = 65_536;
/// How many of its own commits per branch the tool remembers.
const PUSHED_KEPT: usize = 50;
const PUSHED_RECORD_FILE: &str = "pushed.json";
const ACCEPT_JSON: &str = "application/vnd.github+json";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// How many changed paths a reply lists; the counts it gives are always exact.
const PATHS_SHOWN: usize = 50;

static BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*(/[A-Za-z0-9][A-Za-z0-9._-]*)*$").unwrap()
});

pub struct GithubWriteTool {
    repos: Vec<String>,
    api_base: String,
    clones: PathBuf,
    records: PathBuf,
}

impl GithubWriteTool {
    pub fn new(repos: Vec<String>) -> Self {
        Self {
            repos,
            api_base: API_HOST_URL.to_string(),
            clones: clone_root(),
            records: records_root(),
        }
    }

    #[cfg(test)]
    fn for_test(repos: Vec<String>, api_base: String, clones: PathBuf, records: PathBuf) -> Self {
        Self {
            repos,
            api_base,
            clones,
            records,
        }
    }
}

#[async_trait]
impl HqTool for GithubWriteTool {
    fn name(&self) -> &str {
        "github_write"
    }

    fn description(&self) -> &str {
        "Publish the changes made in a github_clone checkout (one fetched without git) as a branch \
         named hq/<branch> and open a pull request for review. Only repositories the owner listed \
         in github_write.repos; never another branch, never a merge, never .github/. Files and \
         text that look like credentials are refused. Push again to the same branch to add a fix \
         after CI fails."
    }

    fn category(&self) -> &str {
        "github"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": {"type": "string", "description": "owner/name, as given to github_clone"},
                "branch": {"type": "string", "description": "Branch name after hq/, for example fix-plant-log"},
                "message": {"type": "string", "description": "Commit message"},
                "open_pr": {"type": "boolean", "description": "Open a pull request if none is open for the branch (default true)"},
                "title": {"type": "string", "description": "Pull request title (default: the message's first line)"},
                "body": {"type": "string", "description": "Pull request description"}
            },
            "required": ["repo", "branch", "message"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let request = Request::parse(&args, &self.repos)?;
        let token = std::env::var(TOKEN_ENV)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .with_context(|| format!("github_write needs a token in {TOKEN_ENV}"))?;
        self.run(request, token).await
    }
}

impl GithubWriteTool {
    async fn run(&self, request: Request, token: String) -> Result<Value> {
        let mut secrets = env_secrets();
        secrets.push(token.clone());
        request.refuse_secret(&secrets)?;
        let checkout = self.clones.join(&request.repo);
        let records = self.records.join(&request.repo);
        let _claim = InFlight::claim(&checkout)?;
        let base = load_base(&records)
            .filter(|b| b.repo == request.repo)
            .with_context(|| {
                format!(
                    "no github_clone checkout of {} made without git; fetch it first",
                    request.repo
                )
            })?;
        let found = {
            let (checkout, clones, base) = (checkout.clone(), self.clones.clone(), base.clone());
            tokio::task::spawn_blocking(move || changes(&checkout, &clones, &base, &secrets))
                .await??
        };
        if found.is_empty() {
            bail!("nothing changed since commit {}", base.commit);
        }
        let api = Api::new(&self.api_base, &request.repo, token)?;
        let pushed = publish(&api, &request, &base, &found, &records).await?;
        // Even when nothing new was pushed: an earlier call may have pushed and then failed to open it.
        let pr = match request.open_pr {
            true => Some(open_pr(&api, &request).await?),
            false => None,
        };
        Ok(reply(&request, pushed.as_ref(), &found, pr))
    }
}

/// A validated call: everything here may be published.
struct Request {
    repo: String,
    branch: String,
    message: String,
    title: String,
    body: String,
    open_pr: bool,
}

impl Request {
    fn parse(args: &Value, allowed: &[String]) -> Result<Self> {
        let repo = repo(args)?;
        if !allowed.iter().any(|r| r.eq_ignore_ascii_case(&repo)) {
            bail!(
                "github_write may push only to the repositories in github_write.repos, and {repo} is not one"
            );
        }
        let name = text(args, "branch").context("`branch` is required")?;
        let name = name.strip_prefix(BRANCH_PREFIX).unwrap_or(name);
        check_branch(name)?;
        let message = text(args, "message")
            .context("`message` is required")?
            .to_string();
        let first_line = message.lines().next().unwrap_or_default().to_string();
        let request = Self {
            repo,
            branch: format!("{BRANCH_PREFIX}{name}"),
            title: text(args, "title").map_or(first_line, str::to_string),
            body: text(args, "body").unwrap_or_default().to_string(),
            open_pr: args.get("open_pr").and_then(Value::as_bool).unwrap_or(true),
            message,
        };
        let limits = [
            ("message", &request.message, MAX_MESSAGE_CHARS),
            ("title", &request.title, MAX_TITLE_CHARS),
            ("body", &request.body, MAX_BODY_CHARS),
        ];
        for (field, value, max) in limits {
            if value.chars().count() > max {
                bail!("`{field}` is longer than {max} characters");
            }
        }
        request.refuse_secret(&[])?;
        Ok(request)
    }

    /// The branch, message, title and body are published too, so they are scanned like files.
    fn refuse_secret(&self, secrets: &[String]) -> Result<()> {
        let fields = [
            ("branch", &self.branch),
            ("message", &self.message),
            ("title", &self.title),
            ("body", &self.body),
        ];
        for (name, value) in fields {
            let leaked = secrets
                .iter()
                .any(|s| !s.is_empty() && value.contains(s.as_str()));
            if leaked || hq_core::redact::looks_like_credential(value) {
                bail!("`{name}` holds something shaped like a credential; nothing was pushed");
            }
        }
        Ok(())
    }
}

/// Values of this process's secret-named variables, refused verbatim wherever they appear,
/// whatever their shape. Short values are left out: they would match ordinary text.
fn env_secrets() -> Vec<String> {
    const SECRET_NAME_ENDINGS: [&str; 5] = ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_PAT"];
    const MIN_SECRET_CHARS: usize = 12;
    std::env::vars()
        .filter(|(name, value)| {
            let name = name.to_ascii_uppercase();
            SECRET_NAME_ENDINGS.iter().any(|e| name.ends_with(e)) && value.len() >= MIN_SECRET_CHARS
        })
        .map(|(_, value)| value)
        .collect()
}

/// Plain names joined by `/`, each one a name git accepts for a branch component.
fn check_branch(name: &str) -> Result<()> {
    let parts_ok = name
        .split('/')
        .all(|part| !part.ends_with(".lock") && !part.ends_with('.'));
    let ok =
        BRANCH.is_match(name) && name.len() <= MAX_BRANCH_CHARS && !name.contains("..") && parts_ok;
    if !ok {
        bail!("`branch` must be plain names joined by /, such as fix-plant-log");
    }
    Ok(())
}

/// GitHub's REST API with the write token, for one repository. The token goes only to the API
/// base this tool was built with, and redirects are not followed, so it never goes elsewhere.
struct Api {
    client: reqwest::Client,
    base: String,
    repo: String,
    token: String,
}

impl Api {
    fn new(base: &str, repo: &str, token: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base: base.to_string(),
            repo: repo.to_string(),
            token,
        })
    }

    /// The response body for a status in `ok`, `None` for a 404, an error for anything else.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        ok: &[u16],
    ) -> Result<Option<Value>> {
        let url = match path {
            "" => format!("{}repos/{}", self.base, self.repo),
            _ => format!("{}repos/{}/{path}", self.base, self.repo),
        };
        let mut req = self
            .client
            .request(method.clone(), &url)
            .header("Accept", ACCEPT_JSON)
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let value: Value = resp.json().await.unwrap_or(Value::Null);
        if ok.contains(&status) {
            return Ok(Some(value));
        }
        if status == 404 {
            return Ok(None);
        }
        let said = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no message");
        bail!("GitHub refused {method} {path} with {status}: {said}")
    }

    async fn need(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        ok: &[u16],
    ) -> Result<Value> {
        self.call(method, path, body, ok)
            .await?
            .with_context(|| format!("GitHub has no {path} in {}", self.repo))
    }

    async fn tree_of(&self, commit: &str) -> Result<String> {
        let found = self
            .need(Method::GET, &format!("git/commits/{commit}"), None, &[200])
            .await?;
        Ok(str_field(&found, "/tree/sha")?.to_string())
    }
}

fn str_field<'a>(value: &'a Value, pointer: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .with_context(|| format!("GitHub's answer has no {pointer}"))
}

struct Pushed {
    commit: String,
    created_branch: bool,
}

/// Where the branch is now, refusing one whose head is not a commit this tool made there.
async fn branch_head(api: &Api, req: &Request, records: &Path) -> Result<Option<String>> {
    let head = api
        .call(
            Method::GET,
            &format!("git/ref/heads/{}", req.branch),
            None,
            &[200],
        )
        .await?;
    let Some(head) = head else { return Ok(None) };
    let head = str_field(&head, "/object/sha")?.to_string();
    let ours = read_pushed(records);
    if !ours
        .get(&req.branch)
        .is_some_and(|shas| shas.contains(&head))
    {
        bail!(
            "{} has commits this tool did not push; use a new branch name",
            req.branch
        );
    }
    Ok(Some(head))
}

/// Blobs, a tree on the base commit's tree, a commit and the branch. `None` when the result is
/// exactly what the branch already holds, so a repeated call adds no empty commit.
async fn publish(
    api: &Api,
    req: &Request,
    base: &Base,
    found: &[Change],
    records: &Path,
) -> Result<Option<Pushed>> {
    let base_tree = api.tree_of(&base.commit).await?;
    let head = branch_head(api, req, records).await?;
    let tree = tree_entries(api, found).await?;
    let new_tree = api
        .need(
            Method::POST,
            "git/trees",
            Some(json!({"base_tree": base_tree, "tree": tree})),
            &[201],
        )
        .await?;
    let new_tree = str_field(&new_tree, "/sha")?.to_string();
    if let Some(head) = &head
        && api.tree_of(head).await? == new_tree
    {
        return Ok(None);
    }
    let parent = head.clone().unwrap_or_else(|| base.commit.clone());
    let body = json!({"message": req.message, "tree": new_tree, "parents": [parent]});
    let commit = api
        .need(Method::POST, "git/commits", Some(body), &[201])
        .await?;
    let commit = str_field(&commit, "/sha")?.to_string();
    // Recorded before the branch moves: an entry for a commit that never lands is harmless,
    // while a branch moved without one would be refused as someone else's on the next call.
    remember_pushed(records, &req.branch, &commit)?;
    move_branch(api, req, &commit, head.is_some()).await?;
    Ok(Some(Pushed {
        commit,
        created_branch: head.is_none(),
    }))
}

async fn move_branch(api: &Api, req: &Request, commit: &str, exists: bool) -> Result<()> {
    if exists {
        let body = json!({"sha": commit, "force": false});
        api.need(
            Method::PATCH,
            &format!("git/refs/heads/{}", req.branch),
            Some(body),
            &[200],
        )
        .await?;
    } else {
        let body = json!({"ref": format!("refs/heads/{}", req.branch), "sha": commit});
        api.need(Method::POST, "git/refs", Some(body), &[201])
            .await?;
    }
    Ok(())
}

async fn tree_entries(api: &Api, found: &[Change]) -> Result<Vec<Value>> {
    let mut tree = Vec::with_capacity(found.len());
    for change in found {
        let sha = match &change.content {
            Some(bytes) => {
                let content = base64::engine::general_purpose::STANDARD.encode(bytes);
                let body = json!({"content": content, "encoding": "base64"});
                let blob = api
                    .need(Method::POST, "git/blobs", Some(body), &[201])
                    .await?;
                Value::String(str_field(&blob, "/sha")?.to_string())
            }
            None => Value::Null,
        };
        tree.push(json!({"path": change.path, "mode": change.mode, "type": "blob", "sha": sha}));
    }
    Ok(tree)
}

/// The open pull request for the branch, or a new one against the default branch.
async fn open_pr(api: &Api, req: &Request) -> Result<String> {
    let owner = req.repo.split('/').next().unwrap_or_default();
    let query = format!("pulls?state=open&head={owner}:{}", req.branch);
    let open = api.need(Method::GET, &query, None, &[200]).await?;
    if let Some(url) = open.pointer("/0/html_url").and_then(Value::as_str) {
        return Ok(url.to_string());
    }
    let info = api.need(Method::GET, "", None, &[200]).await?;
    let default_branch = str_field(&info, "/default_branch")?;
    let body =
        json!({"title": req.title, "head": req.branch, "base": default_branch, "body": req.body});
    let pr = api.need(Method::POST, "pulls", Some(body), &[201]).await?;
    Ok(str_field(&pr, "/html_url")?.to_string())
}

type PushedRecord = std::collections::BTreeMap<String, Vec<String>>;

fn read_pushed(records: &Path) -> PushedRecord {
    std::fs::read(records.join(PUSHED_RECORD_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn remember_pushed(records: &Path, branch: &str, commit: &str) -> Result<()> {
    let mut all = read_pushed(records);
    let shas = all.entry(branch.to_string()).or_default();
    shas.push(commit.to_string());
    let excess = shas.len().saturating_sub(PUSHED_KEPT);
    shas.drain(..excess);
    std::fs::create_dir_all(records)?;
    let tmp = records.join(format!(
        "{PUSHED_RECORD_FILE}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
    std::fs::rename(&tmp, records.join(PUSHED_RECORD_FILE))?;
    Ok(())
}

fn reply(req: &Request, pushed: Option<&Pushed>, found: &[Change], pr: Option<String>) -> Value {
    let Some(pushed) = pushed else {
        let note = "the branch already holds exactly these changes";
        return json!({"branch": req.branch, "unchanged": true, "note": note, "pull_request": pr});
    };
    let (deleted, written): (Vec<&Change>, Vec<&Change>) =
        found.iter().partition(|c| c.content.is_none());
    let names = |list: &[&Change]| {
        list.iter()
            .take(PATHS_SHOWN)
            .map(|c| c.path.clone())
            .collect::<Vec<_>>()
    };
    json!({
        "branch": req.branch,
        "commit": pushed.commit,
        "created_branch": pushed.created_branch,
        "written_count": written.len(),
        "written": names(&written),
        "deleted_count": deleted.len(),
        "deleted": names(&deleted),
        "pull_request": pr,
    })
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
