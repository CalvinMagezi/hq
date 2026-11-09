//! Read-only GitHub access.
//!
//! The tool builds every `gh` command line itself from validated parts, with no shell,
//! so no argument can add a flag or turn a read into a write. The token's own scopes remain
//! the hard limit: use a read-only token.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use crate::registry::HqTool;

pub mod app;
mod changes;
pub mod dispatch;
pub mod repos;
mod tarball;
mod write;

pub use write::GithubWriteTool;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const CLONE_TIMEOUT: Duration = Duration::from_secs(300);
const USER_AGENT: &str = "hq-github-read";
const MAX_OUTPUT_CHARS: usize = 100_000;
/// A diff read for verification may be longer than a tool reply; past this it is cut and says so.
const MAX_DIFF_CHARS: usize = 2_000_000;
/// What `cap` appends when it cuts, so a reader can tell a cut reply from a whole one.
pub(crate) const TRUNCATED_MARK: &str = "\n[output truncated]";
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;
const API_HOST_URL: &str = "https://api.github.com/";
const API_PREFIXES: [&str; 5] = ["repos/", "search/", "orgs/", "users/", "rate_limit"];
const SEARCH_KINDS: [&str; 4] = ["repos", "code", "issues", "prs"];
const WAIT_CI_ACTION: &str = "wait_ci";
const CI_POLL: Duration = Duration::from_secs(20);
/// A workflow can take a while to appear after a push; past this the commit has no CI.
const CI_NO_RUN_GRACE: Duration = Duration::from_secs(120);
/// Under the reply's own time limit, so the worker can still act on the answer.
const CI_MAX_WAIT: Duration = Duration::from_secs(20 * 60);
/// Variables the child may see. The token is the only secret; GH_HOST is deliberately absent.
const CHILD_ENV: [&str; 6] = ["PATH", "HOME", "GH_TOKEN", "GITHUB_TOKEN", "LANG", "TMPDIR"];

static REPO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}$").unwrap());
static ENDPOINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9/_.,:%+=&?-]{1,500}$").unwrap());

/// One validated GitHub call, ready to run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GhCall {
    /// `gh <args>`.
    Gh(Vec<String>),
    /// GET of an API path (also the fallback when `gh` is not installed).
    ApiGet(String),
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn repo(args: &Value) -> Result<String> {
    let r = text(args, "repo").context("`repo` is required, as owner/name")?;
    if !is_valid_repo(r) {
        bail!("`repo` must look like owner/name");
    }
    Ok(r.to_string())
}

fn number(args: &Value) -> Result<String> {
    let n = args
        .get("number")
        .and_then(Value::as_u64)
        .context("`number` is required")?;
    Ok(n.to_string())
}

fn limit(args: &Value) -> String {
    args.get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
        .to_string()
}

/// Free text goes in as one argument; a leading dash would make it a flag.
fn free_text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    let t = text(args, key).with_context(|| format!("`{key}` is required"))?;
    if t.starts_with('-') || t.contains('\0') {
        bail!("`{key}` may not start with '-'");
    }
    Ok(t)
}

fn endpoint(path: &str) -> Result<String> {
    let p = path.trim().trim_start_matches('/');
    let allowed = API_PREFIXES.iter().any(|prefix| p.starts_with(prefix));
    if !allowed || !ENDPOINT.is_match(p) || p.contains("..") {
        bail!("`endpoint` must be a GET path under repos/, search/, orgs/ or users/");
    }
    Ok(p.to_string())
}

fn gh(parts: &[&str]) -> GhCall {
    GhCall::Gh(parts.iter().map(|s| s.to_string()).collect())
}

/// Turns tool arguments into one fixed-shape call, or refuses.
pub(crate) fn build_call(args: &Value) -> Result<GhCall> {
    let action = text(args, "action").context("`action` is required")?;
    let call = match action {
        "repo_view" => GhCall::ApiGet(format!("repos/{}", repo(args)?)),
        "pr_list" | "issue_list" | "release_list" | "run_list" => {
            let noun = action.trim_end_matches("_list");
            gh(&[
                noun,
                "list",
                "--repo",
                &repo(args)?,
                "--limit",
                &limit(args),
            ])
        }
        "pr_view" | "issue_view" => {
            let noun = action.trim_end_matches("_view");
            gh(&[
                noun,
                "view",
                &number(args)?,
                "--repo",
                &repo(args)?,
                "--comments",
            ])
        }
        "pr_diff" => gh(&["pr", "diff", &number(args)?, "--repo", &repo(args)?]),
        "search" => {
            let kind = text(args, "kind").unwrap_or("repos");
            if !SEARCH_KINDS.contains(&kind) {
                bail!("`kind` must be one of repos, code, issues, prs");
            }
            gh(&[
                "search",
                kind,
                free_text(args, "query")?,
                "--limit",
                &limit(args),
            ])
        }
        "file" => {
            let path = free_text(args, "path")?;
            if path.contains("..") || !ENDPOINT.is_match(path) {
                bail!("`path` must be a plain repository path");
            }
            let mut api = format!(
                "repos/{}/contents/{}",
                repo(args)?,
                path.trim_start_matches('/')
            );
            if let Some(r) = text(args, "ref") {
                if r.starts_with('-') || !ENDPOINT.is_match(r) || r.contains("..") {
                    bail!("`ref` must be a plain branch, tag or commit");
                }
                api.push_str(&format!("?ref={r}"));
            }
            GhCall::ApiGet(api)
        }
        "api" => GhCall::ApiGet(endpoint(free_text(args, "endpoint")?)?),
        other => bail!("unknown action `{other}`; this tool only reads"),
    };
    Ok(call)
}

fn child_env() -> Vec<(String, String)> {
    CHILD_ENV
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .chain([
            ("GH_PROMPT_DISABLED".into(), "1".into()),
            ("NO_COLOR".into(), "1".into()),
        ])
        .collect()
}

fn cap(s: String) -> String {
    cap_to(s, MAX_OUTPUT_CHARS)
}

fn cap_to(mut s: String, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        s = s.chars().take(max_chars).collect();
        s.push_str(TRUNCATED_MARK);
    }
    s
}

async fn run(program: &str, args: &[String], timeout: Duration) -> Result<String> {
    run_capped(program, args, timeout, MAX_OUTPUT_CHARS).await
}

async fn run_capped(
    program: &str,
    args: &[String],
    timeout: Duration,
    max_chars: usize,
) -> Result<String> {
    let child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(child_env())
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("could not start {program}"))?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("{program} timed out"))??;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(cap_to(stdout, max_chars));
    }
    bail!(
        "{program} failed: {}",
        cap(String::from_utf8_lossy(&out.stderr).into_owned())
    )
}

/// A GET carrying the read-only token when one is set. The write token is never sent from here.
fn read_request(client: &reqwest::Client, url: &str, accept: &str) -> reqwest::RequestBuilder {
    let req = client.get(url).header("Accept", accept);
    match std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        Ok(token) => req.bearer_auth(token),
        Err(_) => req,
    }
}

/// No `gh` on this machine: public data can still be read over HTTPS.
async fn http_get(path: &str) -> Result<String> {
    http_get_accepting(path, "application/vnd.github+json", MAX_OUTPUT_CHARS).await
}

async fn http_get_accepting(path: &str, accept: &str, max_chars: usize) -> Result<String> {
    http_get_as(path, accept, max_chars, None).await
}

async fn http_get_as(
    path: &str,
    accept: &str,
    max_chars: usize,
    token: Option<&str>,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(COMMAND_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()?;
    let url = format!("{API_HOST_URL}{path}");
    let resp = tarball::authed(&client, &url, accept, token).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        bail!("GitHub returned {status}: {}", cap(body));
    }
    Ok(cap_to(body, max_chars))
}

/// The read tools, plus `github_write`, which refuses every call until the owner lists
/// repositories in `github_write.repos`.
pub fn create_github_tools(write_repos: &[String]) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(GithubReadTool),
        Box::new(GithubCloneTool),
        Box::new(GithubWriteTool::new(write_repos.to_vec())),
    ]
}

pub struct GithubReadTool;

#[async_trait]
impl HqTool for GithubReadTool {
    fn name(&self) -> &str {
        "github_read"
    }

    fn description(&self) -> &str {
        "Read GitHub without changing anything: repo_view, pr_list, pr_view, pr_diff, issue_list, \
         issue_view, release_list, run_list, search (kind: repos, code, issues, prs; give repo to search a private one), \
         file (repo, path, optional ref), api (a GET path under repos/, search/, orgs/ or users/) and wait_ci (repo, ref: \
         waits for CI on that branch or commit and answers passed, failed, none or running). Uses the gh CLI when \
         installed, otherwise HTTPS with HQ's GitHub App for repositories it covers. Cannot create, edit, comment or merge."
    }

    fn category(&self) -> &str {
        "web"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["repo_view", "pr_list", "pr_view", "pr_diff", "issue_list", "issue_view", "release_list", "run_list", "search", "file", "api", "wait_ci"]},
                "repo": {"type": "string", "description": "owner/name"},
                "number": {"type": "integer", "description": "PR or issue number"},
                "query": {"type": "string"},
                "kind": {"type": "string", "enum": ["repos", "code", "issues", "prs"]},
                "path": {"type": "string", "description": "File path inside the repo, for action file"},
                "ref": {"type": "string", "description": "Branch, tag or commit, for action file"},
                "endpoint": {"type": "string", "description": "GET path for action api, for example repos/owner/name/contents/README.md"},
                "limit": {"type": "integer", "description": "Max rows for list and search actions (default 20, max 100)"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        if text(&args, "action") == Some(WAIT_CI_ACTION) {
            return wait_ci(&args).await;
        }
        let call = build_call(&args)?;
        let have_gh = which::which("gh").is_ok();
        let output = match (call, have_gh) {
            (GhCall::Gh(argv), true) => run("gh", &argv, COMMAND_TIMEOUT).await?,
            (GhCall::ApiGet(path), true) => {
                run(
                    "gh",
                    &["api".into(), "--method".into(), "GET".into(), path],
                    COMMAND_TIMEOUT,
                )
                .await?
            }
            (GhCall::ApiGet(path), false) => read_without_gh(&path).await?,
            (GhCall::Gh(_), false) => {
                let path = rest_path(&args)
                    .with_context(|| "this action needs `repo`, and `number` where it applies")?;
                read_rest(&args, &path).await?
            }
        };
        Ok(json!({ "output": output }))
    }
}

/// The REST path of an action `gh` would serve, for a phone without it.
fn rest_path(args: &Value) -> Option<String> {
    let action = text(args, "action")?;
    if action == "search" {
        return search_path(args);
    }
    let repo = repo(args).ok()?;
    let per = limit(args);
    Some(match action {
        "pr_list" => format!("repos/{repo}/pulls?state=open&per_page={per}"),
        "issue_list" => format!("repos/{repo}/issues?state=open&per_page={per}"),
        "release_list" => format!("repos/{repo}/releases?per_page={per}"),
        "run_list" => format!("repos/{repo}/actions/runs?per_page={per}"),
        "pr_view" | "pr_diff" => format!("repos/{repo}/pulls/{}", number(args).ok()?),
        "issue_view" => format!("repos/{repo}/issues/{}", number(args).ok()?),
        _ => return None,
    })
}

/// A search over REST. `repo`, when given, narrows it and lets a private repository be searched.
fn search_path(args: &Value) -> Option<String> {
    let query = free_text(args, "query").ok()?;
    let (endpoint, qualifier) = match text(args, "kind").unwrap_or("repos") {
        "repos" => ("repositories", ""),
        "code" => ("code", ""),
        "issues" => ("issues", " is:issue"),
        "prs" => ("issues", " is:pr"),
        _ => return None,
    };
    let scope = repo(args).map(|r| format!(" repo:{r}")).unwrap_or_default();
    let full = format!("{query}{qualifier}{scope}");
    let q: String = url::form_urlencoded::byte_serialize(full.as_bytes()).collect();
    Some(format!("search/{endpoint}?q={q}&per_page={}", limit(args)))
}

async fn read_rest(args: &Value, path: &str) -> Result<String> {
    let accept = match text(args, "action") {
        Some("pr_diff") => "application/vnd.github.diff",
        _ => "application/vnd.github+json",
    };
    let app = app::AppClient::standard()?;
    let token = match path_repo(path).or_else(|| repo(args).ok()) {
        Some(repo) => app_token(&app, &repo, step_for_path(path)).await?,
        None => None,
    };
    let secret = token.as_ref().map(app::AppToken::secret);
    http_get_as(path, accept, MAX_OUTPUT_CHARS, secret).await
}

async fn read_api(path: &str) -> Result<String> {
    if which::which("gh").is_ok() {
        let argv = ["api".into(), "--method".into(), "GET".into(), path.into()];
        return run("gh", &argv, COMMAND_TIMEOUT).await;
    }
    read_without_gh(path).await
}

#[derive(Debug, PartialEq, Eq)]
enum CiState {
    NoRuns,
    Running,
    Passed,
    Failed,
}

/// Where a commit's workflow runs stand, from the runs list.
fn ci_state(runs: &[Value]) -> CiState {
    const GOOD: [&str; 3] = ["success", "skipped", "neutral"];
    if runs.is_empty() {
        return CiState::NoRuns;
    }
    if runs.iter().any(|r| r["status"] != "completed") {
        return CiState::Running;
    }
    let passed = runs
        .iter()
        .all(|r| r["conclusion"].as_str().is_some_and(|c| GOOD.contains(&c)));
    if passed {
        CiState::Passed
    } else {
        CiState::Failed
    }
}

/// Waits for CI on a branch or commit inside the tool, so the wait costs no model calls.
async fn wait_ci(args: &Value) -> Result<Value> {
    let repo = repo(args)?;
    let git_ref = free_text(args, "ref")?;
    if !ENDPOINT.is_match(git_ref) || git_ref.contains("..") {
        bail!("`ref` must be a plain branch, tag or commit");
    }
    let commit: Value =
        serde_json::from_str(&read_api(&format!("repos/{repo}/commits/{git_ref}")).await?)?;
    let sha = commit["sha"]
        .as_str()
        .context("GitHub returned no commit")?;
    let started = std::time::Instant::now();
    let path = format!("repos/{repo}/actions/runs?head_sha={sha}&per_page={MAX_LIMIT}");
    loop {
        let body: Value = serde_json::from_str(&read_api(&path).await?)?;
        let runs = body["workflow_runs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let waited = started.elapsed();
        let state = ci_state(&runs);
        let verdict = match state {
            CiState::Passed => Some("passed"),
            CiState::Failed => Some("failed"),
            CiState::NoRuns if waited >= CI_NO_RUN_GRACE => Some("none"),
            _ if waited >= CI_MAX_WAIT => Some("running"),
            _ => None,
        };
        if let Some(ci) = verdict {
            let runs: Vec<Value> = runs
                .iter()
                .map(|r| json!({"id": r["id"], "name": r["name"], "conclusion": r["conclusion"]}))
                .collect();
            return Ok(json!({ "ci": ci, "commit": sha, "runs": runs }));
        }
        tokio::time::sleep(CI_POLL).await;
    }
}

/// `owner/name` of a `repos/...` API path.
fn path_repo(path: &str) -> Option<String> {
    let mut parts = path.strip_prefix("repos/")?.split(['/', '?']);
    let repo = format!("{}/{}", parts.next()?, parts.next()?);
    is_valid_repo(&repo).then_some(repo)
}

/// The least a token needs for this path: CI reads are their own permission.
fn step_for_path(path: &str) -> app::Step {
    let ci = ["/actions/", "/check-runs", "/check-suites", "/status"];
    match ci.iter().any(|part| path.contains(part)) {
        true => app::Step::ReadCi,
        false => app::Step::Fetch,
    }
}

/// With an App, a token for just that repository and kind of read; otherwise public data.
async fn read_without_gh(path: &str) -> Result<String> {
    let app = app::AppClient::standard()?;
    let token = match path_repo(path) {
        Some(repo) => app_token(&app, &repo, step_for_path(path)).await?,
        None => None,
    };
    let secret = token.as_ref().map(app::AppToken::secret);
    http_get_as(
        path,
        "application/vnd.github+json",
        MAX_OUTPUT_CHARS,
        secret,
    )
    .await
}

/// `owner/name`, as the tool validates a repository. Neither part may start with a dot: `o/.`
/// would name the owner folder of the clone root (and the refresh would delete it), `../x` its
/// parent, and GitHub allows neither as an owner or a repository name.
pub(crate) fn is_valid_repo(repo: &str) -> bool {
    REPO.is_match(repo)
        && !repo.starts_with('-')
        && !repo.contains("..")
        && repo.split('/').all(|part| !part.starts_with('.'))
}

/// GET of an API path read for verification: `gh` when installed, otherwise public data over
/// HTTPS. The path is checked the way the tool checks `endpoint`.
pub(crate) async fn api_get(path: &str) -> Result<String> {
    let path = endpoint(path)?;
    if which::which("gh").is_ok() {
        run(
            "gh",
            &["api".into(), "--method".into(), "GET".into(), path],
            COMMAND_TIMEOUT,
        )
        .await
    } else {
        http_get(&path).await
    }
}

/// The unified diff of a pull request and whether it was cut. `gh` when installed, otherwise
/// the public API's diff media type.
pub(crate) async fn pr_diff_text(repo: &str, number: u64) -> Result<(String, bool)> {
    if !is_valid_repo(repo) {
        bail!("repository must look like owner/name");
    }
    let text = if which::which("gh").is_ok() {
        run_capped(
            "gh",
            &[
                "pr".into(),
                "diff".into(),
                number.to_string(),
                "--repo".into(),
                repo.to_string(),
            ],
            COMMAND_TIMEOUT,
            MAX_DIFF_CHARS,
        )
        .await?
    } else {
        http_get_accepting(
            &format!("repos/{repo}/pulls/{number}"),
            "application/vnd.github.diff",
            MAX_DIFF_CHARS,
        )
        .await?
    };
    let truncated = text.ends_with(TRUNCATED_MARK);
    Ok((text, truncated))
}

pub struct GithubCloneTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloneMethod {
    Gh,
    Git,
    /// Neither tool is installed (a phone): the commit's tarball over HTTPS.
    Tarball,
}

fn clone_method(have_gh: bool, have_git: bool) -> CloneMethod {
    match (have_gh, have_git) {
        (true, _) => CloneMethod::Gh,
        (false, true) => CloneMethod::Git,
        (false, false) => CloneMethod::Tarball,
    }
}

/// Clones in flight, by destination, so two calls for one repo never interleave.
static IN_FLIGHT: LazyLock<std::sync::Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);

/// Holds a destination for one clone call and frees it when dropped.
struct InFlight(PathBuf);

impl InFlight {
    fn claim(dest: &Path) -> Result<Self> {
        let mut set = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(dest.to_path_buf()) {
            bail!(
                "{} is in use by another GitHub call; try again when that call finishes",
                dest.display()
            );
        }
        Ok(Self(dest.to_path_buf()))
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// The reply for a checkout already on disk, or `None` to clone. A tarball checkout is reused
/// only with its base record; one with neither a record nor `.git` is a fetch that was cut
/// short, and since the clone folder is this tool's scratch space it is removed and fetched again.
fn cached_reply(dest: &Path, records: &Path, method: CloneMethod) -> Result<Option<Value>> {
    if !dest.exists() {
        return Ok(None);
    }
    let path = dest.display().to_string();
    if method != CloneMethod::Tarball || dest.join(".git").exists() {
        return Ok(Some(json!({ "path": path, "cached": true })));
    }
    if let Some(base) = tarball::load_base(records) {
        let reply =
            json!({ "path": path, "cached": true, "via": "tarball", "commit": base.commit });
        return Ok(Some(reply));
    }
    std::fs::remove_dir_all(dest).with_context(|| format!("could not clear {path}"))?;
    Ok(None)
}

fn tarball_reply(path: &str, fetched: &tarball::Fetched) -> Value {
    let shown: Vec<&String> = fetched
        .skipped
        .iter()
        .take(tarball::SKIPPED_SHOWN)
        .collect();
    json!({
        "path": path,
        "cached": false,
        "via": "tarball",
        "commit": fetched.base.commit,
        "files": fetched.base.files.len(),
        "skipped_count": fetched.skipped.len(),
        "skipped": shown,
    })
}

/// Where this tool keeps what it knows about each checkout (its base commit, the commits
/// `github_write` pushed). File tools may not read or write here (`governance::secrets`), so a
/// worker cannot rewrite the records its own pushes are checked against.
pub fn records_root() -> PathBuf {
    hq_core::config::HqConfig::hq_dir().join("github")
}

/// A read token for `repo` from the App. `None` when the App is absent or does not cover it, so
/// a public repository can still be read without one.
async fn app_token(
    app: &app::AppClient,
    repo: &str,
    step: app::Step,
) -> Result<Option<app::AppToken>> {
    if app.store().load().is_none() {
        return Ok(None);
    }
    match app.token(repo, step).await {
        Ok(token) => Ok(Some(token)),
        // Only "not one of yours" may fall through to a public read; "Reconnect" must be heard.
        Err(e) if matches!(e.downcast_ref(), Some(app::TokenRefused::NotCovered(_))) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Removes a checkout and its base record so the next fetch starts from the default branch. The
/// record of pushed commits stays: a later push still knows which `hq/` branches are its own.
fn discard_checkout(dest: &Path, records: &Path) -> Result<()> {
    match std::fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(dest)?,
        Ok(_) => std::fs::remove_file(dest)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", dest.display())),
    }
    let _ = std::fs::remove_file(tarball::base_record_path(records));
    Ok(())
}

/// Creates the clone folder readable by this user only. In a shared temp directory another user
/// could have made it first; setting its mode then fails, and nothing is cloned into it.
fn make_private_root(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        const OWNER_ONLY: u32 = 0o700;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(OWNER_ONLY))
            .with_context(|| format!("{} is not this user's to clone into", root.display()))?;
    }
    Ok(())
}

/// Shallow checkouts land here so a repo can be searched with grep and read_file.
pub fn clone_root() -> PathBuf {
    std::env::temp_dir().join("hq-github-clones")
}

#[async_trait]
impl HqTool for GithubCloneTool {
    fn name(&self) -> &str {
        "github_clone"
    }

    fn description(&self) -> &str {
        "Shallow-clone a GitHub repo (owner/name) into a scratch folder and return its path, so it can be \
         searched with grep and read_file. Without git or gh installed it downloads the default branch's tarball \
         instead (no history, links and .git entries left out); github_write can publish changes made in such a \
         checkout. An existing checkout is reused as it is, edits included; pass refresh: true to discard it and \
         fetch the default branch again, as at the start of a new task."
    }

    fn category(&self) -> &str {
        "web"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": {"type": "string", "description": "owner/name"},
                "refresh": {"type": "boolean", "description": "Discard the existing checkout, edits included, and fetch again (default false)"}
            },
            "required": ["repo"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let repo = repo(&args)?;
        let dest = clone_root().join(&repo);
        let records = records_root().join(&repo);
        let _claim = InFlight::claim(&dest)?;
        if args
            .get("refresh")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            discard_checkout(&dest, &records)?;
        }
        let app = app::AppClient::standard()?;
        // github_write publishes only a tarball checkout, so a connected App always fetches one.
        let method = match app.store().load() {
            Some(_) => CloneMethod::Tarball,
            None => clone_method(which::which("gh").is_ok(), which::which("git").is_ok()),
        };
        if let Some(reply) = cached_reply(&dest, &records, method)? {
            return Ok(reply);
        }
        make_private_root(&clone_root())?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let dest_arg = dest.display().to_string();
        // A base record left for a git checkout would describe files git did not fetch.
        let _ = std::fs::remove_file(tarball::base_record_path(&records));
        if method == CloneMethod::Tarball {
            let token = app_token(&app, &repo, app::Step::Fetch).await?;
            let secret = token.as_ref().map(app::AppToken::secret);
            let fetched = tarball::fetch(API_HOST_URL, &repo, &dest, &records, secret).await?;
            return Ok(tarball_reply(&dest_arg, &fetched));
        }
        if method == CloneMethod::Gh {
            let argv = [
                "repo",
                "clone",
                repo.as_str(),
                dest_arg.as_str(),
                "--",
                "--depth",
                "1",
                "--no-tags",
            ]
            .map(String::from);
            run("gh", &argv, CLONE_TIMEOUT).await?;
        } else {
            let url = format!("https://github.com/{repo}.git");
            let argv = [
                "clone",
                "--depth",
                "1",
                "--no-tags",
                "--",
                url.as_str(),
                dest_arg.as_str(),
            ]
            .map(String::from);
            run("git", &argv, CLONE_TIMEOUT).await?;
        }
        Ok(json!({ "path": dest_arg, "cached": false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(v: Value) -> Result<GhCall> {
        build_call(&v)
    }

    /// `o/.` joined to the clone root is the owner folder, which a refresh or a cut-short
    /// tarball fetch removes with everything in it.
    #[test]
    fn a_repo_part_that_is_or_starts_with_a_dot_is_refused() {
        for bad in ["o/.", "./r", "o/..", "../r", "o/.git", ".o/r", "o/.hidden"] {
            assert!(!is_valid_repo(bad), "{bad}");
            assert!(repo(&json!({ "repo": bad })).is_err(), "{bad}");
        }
        for good in ["o/r", "o/r.js", "o.io/r", "o-x/r_y", "o/r."] {
            assert!(is_valid_repo(good), "{good}");
        }
    }

    #[test]
    fn a_tarball_checkout_is_reused_only_with_its_base_record() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("o/r");
        let records = root.path().join("records/o/r");
        std::fs::create_dir_all(&records).unwrap();
        assert!(
            cached_reply(&dest, &records, CloneMethod::Tarball)
                .unwrap()
                .is_none()
        );

        std::fs::create_dir_all(dest.join(".git")).unwrap();
        let git = cached_reply(&dest, &records, CloneMethod::Git)
            .unwrap()
            .unwrap();
        assert_eq!(git["cached"], true);
        // A git checkout stays usable after git is gone from the machine.
        let kept = cached_reply(&dest, &records, CloneMethod::Tarball)
            .unwrap()
            .unwrap();
        assert_eq!(kept["cached"], true);
        std::fs::remove_dir(dest.join(".git")).unwrap();

        let base = tarball::Base {
            repo: "o/r".into(),
            commit: "a".repeat(40),
            files: Vec::new(),
        };
        let record = tarball::base_record_path(&records);
        std::fs::write(&record, serde_json::to_vec(&base).unwrap()).unwrap();
        let reply = cached_reply(&dest, &records, CloneMethod::Tarball)
            .unwrap()
            .unwrap();
        assert_eq!(reply["commit"], "a".repeat(40));

        std::fs::write(&record, b"{ truncated").unwrap();
        assert!(
            cached_reply(&dest, &records, CloneMethod::Tarball)
                .unwrap()
                .is_none()
        );
        assert!(
            !dest.exists(),
            "a checkout without a usable record is cleared to fetch again"
        );
    }

    #[test]
    fn a_refresh_drops_the_checkout_and_its_base_but_keeps_the_pushed_record() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("clones/o/r");
        let records = root.path().join("records/o/r");
        std::fs::create_dir_all(dest.join("src")).unwrap();
        std::fs::write(dest.join("src/edited.rs"), b"work in progress").unwrap();
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(tarball::base_record_path(&records), b"{}").unwrap();
        std::fs::write(records.join("pushed.json"), b"{}").unwrap();

        discard_checkout(&dest, &records).unwrap();
        assert!(!dest.exists());
        assert!(!tarball::base_record_path(&records).exists());
        assert!(records.join("pushed.json").exists());
        discard_checkout(&dest, &records).unwrap();
    }

    #[test]
    fn one_destination_is_cloned_by_one_call_at_a_time() {
        let dest = Path::new("/nowhere/o/r-in-flight-test");
        let first = InFlight::claim(dest).unwrap();
        assert!(InFlight::claim(dest).is_err());
        drop(first);
        assert!(InFlight::claim(dest).is_ok());
    }

    #[test]
    fn clone_prefers_gh_then_git_then_the_tarball() {
        assert_eq!(clone_method(true, true), CloneMethod::Gh);
        assert_eq!(clone_method(true, false), CloneMethod::Gh);
        assert_eq!(clone_method(false, true), CloneMethod::Git);
        assert_eq!(clone_method(false, false), CloneMethod::Tarball);
    }

    #[test]
    fn reads_build_the_exact_argv() {
        assert_eq!(
            call(json!({"action": "pr_view", "repo": "o/r", "number": 7})).unwrap(),
            gh(&["pr", "view", "7", "--repo", "o/r", "--comments"])
        );
        assert_eq!(
            call(json!({"action": "issue_list", "repo": "o/r", "limit": 500})).unwrap(),
            gh(&["issue", "list", "--repo", "o/r", "--limit", "100"])
        );
        assert_eq!(
            call(json!({"action": "api", "endpoint": "/repos/o/r/contents/README.md?ref=main"}))
                .unwrap(),
            GhCall::ApiGet("repos/o/r/contents/README.md?ref=main".into())
        );
        assert_eq!(
            call(json!({"action": "file", "repo": "o/r", "path": "src/lib.rs", "ref": "v1.0"}))
                .unwrap(),
            GhCall::ApiGet("repos/o/r/contents/src/lib.rs?ref=v1.0".into())
        );
    }

    #[test]
    fn every_write_shaped_request_is_refused() {
        for action in [
            "pr_create",
            "pr_merge",
            "pr_close",
            "issue_create",
            "issue_comment",
            "repo_create",
            "repo_delete",
            "release_create",
            "secret_set",
            "workflow_run",
            "pr_review",
            "gist_create",
            "auth_login",
        ] {
            assert!(
                call(json!({"action": action, "repo": "o/r", "number": 1})).is_err(),
                "{action}"
            );
        }
    }

    #[test]
    fn no_argument_can_become_a_flag_or_a_different_endpoint() {
        for bad in [
            json!({"action": "repo_view", "repo": "--hostname=evil.example/x"}),
            json!({"action": "repo_view", "repo": "o/r; rm -rf /"}),
            json!({"action": "repo_view", "repo": "-o/r"}),
            json!({"action": "search", "query": "--web"}),
            json!({"action": "search", "query": "x", "kind": "repos --web"}),
            json!({"action": "api", "endpoint": "graphql"}),
            json!({"action": "api", "endpoint": "user/repos"}),
            json!({"action": "api", "endpoint": "repos/o/r/../../admin"}),
            json!({"action": "api", "endpoint": "repos/o/r -X DELETE"}),
            json!({"action": "api", "endpoint": "repos/o/r\n--method DELETE"}),
            json!({"action": "file", "repo": "o/r", "path": "../etc/passwd"}),
            json!({"action": "file", "repo": "o/r", "path": "a", "ref": "--upload-pack=x"}),
            json!({"action": "pr_view", "repo": "o/r", "number": "1 --repo x/y"}),
        ] {
            assert!(call(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_child_never_sees_a_host_override() {
        let env: Vec<String> = child_env().into_iter().map(|(k, _)| k).collect();
        assert!(
            !env.iter()
                .any(|k| k == "GH_HOST" || k == "GH_ENTERPRISE_TOKEN")
        );
    }

    #[test]
    fn every_action_has_a_rest_path_for_a_phone_without_gh() {
        let path = |args: Value| rest_path(&args).unwrap();
        assert_eq!(
            path(json!({"action": "pr_list", "repo": "o/r"})),
            "repos/o/r/pulls?state=open&per_page=20"
        );
        assert_eq!(
            path(json!({"action": "pr_diff", "repo": "o/r", "number": 7})),
            "repos/o/r/pulls/7"
        );
        assert_eq!(
            path(json!({"action": "issue_view", "repo": "o/r", "number": 7})),
            "repos/o/r/issues/7"
        );
        assert_eq!(
            path(json!({"action": "release_list", "repo": "o/r", "limit": 5})),
            "repos/o/r/releases?per_page=5"
        );
        assert_eq!(
            path(json!({"action": "search", "kind": "prs", "query": "fix bug", "repo": "o/r"})),
            "search/issues?q=fix+bug+is%3Apr+repo%3Ao%2Fr&per_page=20"
        );
        assert_eq!(
            rest_path(&json!({"action": "pr_view", "repo": "o/r"})),
            None,
            "no number"
        );
    }

    #[test]
    fn ci_is_read_from_every_run_of_the_commit() {
        let run =
            |status: &str, conclusion: Value| json!({"status": status, "conclusion": conclusion});
        assert_eq!(ci_state(&[]), CiState::NoRuns);
        assert_eq!(
            ci_state(&[
                run("completed", json!("success")),
                run("in_progress", Value::Null)
            ]),
            CiState::Running
        );
        assert_eq!(
            ci_state(&[
                run("completed", json!("success")),
                run("completed", json!("skipped"))
            ]),
            CiState::Passed
        );
        assert_eq!(
            ci_state(&[
                run("completed", json!("success")),
                run("completed", json!("failure"))
            ]),
            CiState::Failed
        );
        assert_eq!(ci_state(&[run("completed", Value::Null)]), CiState::Failed);
    }

    #[test]
    fn only_the_reader_is_read_only() {
        assert!(GithubReadTool.is_read_only());
        assert!(!GithubCloneTool.is_read_only());
    }

    /// Network and, optionally, gh: `cargo test -p hq-tools github_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "reads api.github.com; run on demand"]
    async fn github_live_reads_a_public_repo() {
        let view = GithubReadTool
            .execute(json!({"action": "repo_view", "repo": "rust-lang/rust-analyzer"}))
            .await;
        let file = GithubReadTool
            .execute(
                json!({"action": "file", "repo": "rust-lang/rust-analyzer", "path": "README.md"}),
            )
            .await
            .unwrap();
        println!(
            "repo_view ok={} file bytes={}",
            view.is_ok(),
            file["output"].as_str().unwrap_or("").len()
        );
        assert!(
            file["output"]
                .as_str()
                .unwrap_or("")
                .contains("rust-analyzer")
        );
    }
}
