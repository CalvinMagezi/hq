//! Read-only GitHub access.
//!
//! The tool builds every `gh` command line itself from validated parts, with no shell,
//! so no argument can add a flag or turn a read into a write. The token's own scopes remain
//! the hard limit: use a read-only token.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use crate::registry::HqTool;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const CLONE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_OUTPUT_CHARS: usize = 100_000;
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;
const API_HOST_URL: &str = "https://api.github.com/";
const API_PREFIXES: [&str; 5] = ["repos/", "search/", "orgs/", "users/", "rate_limit"];
const SEARCH_KINDS: [&str; 4] = ["repos", "code", "issues", "prs"];
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
    if !REPO.is_match(r) || r.starts_with('-') || r.contains("..") {
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

fn cap(mut s: String) -> String {
    if s.chars().count() > MAX_OUTPUT_CHARS {
        s = s.chars().take(MAX_OUTPUT_CHARS).collect();
        s.push_str("\n[output truncated]");
    }
    s
}

async fn run(program: &str, args: &[String], timeout: Duration) -> Result<String> {
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
        return Ok(cap(stdout));
    }
    bail!(
        "{program} failed: {}",
        cap(String::from_utf8_lossy(&out.stderr).into_owned())
    )
}

/// No `gh` on this machine: public data can still be read over HTTPS.
async fn http_get(path: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(COMMAND_TIMEOUT)
        .user_agent("hq-github-read")
        .build()?;
    let mut req = client
        .get(format!("{API_HOST_URL}{path}"))
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        bail!("GitHub returned {status}: {}", cap(body));
    }
    Ok(cap(body))
}

pub fn create_github_tools() -> Vec<Box<dyn HqTool>> {
    vec![Box::new(GithubReadTool), Box::new(GithubCloneTool)]
}

pub struct GithubReadTool;

#[async_trait]
impl HqTool for GithubReadTool {
    fn name(&self) -> &str {
        "github_read"
    }

    fn description(&self) -> &str {
        "Read GitHub without changing anything: repo_view, pr_list, pr_view, pr_diff, issue_list, \
         issue_view, release_list, run_list, search (kind: repos, code, issues, prs), file (repo, path, optional ref) \
         and api (a GET path under repos/, search/, orgs/ or users/). Uses the gh CLI when installed, otherwise reads \
         public data over HTTPS. Cannot create, edit, comment or merge."
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
                "action": {"type": "string", "enum": ["repo_view", "pr_list", "pr_view", "pr_diff", "issue_list", "issue_view", "release_list", "run_list", "search", "file", "api"]},
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
            (GhCall::ApiGet(path), false) => http_get(&path).await?,
            (GhCall::Gh(_), false) => bail!(
                "the gh CLI is not installed here; repo_view, file and api still work for public data"
            ),
        };
        Ok(json!({ "output": output }))
    }
}

pub struct GithubCloneTool;

/// Shallow checkouts land here so a repo can be searched with grep and read_file.
fn clone_root() -> PathBuf {
    std::env::temp_dir().join("hq-github-clones")
}

#[async_trait]
impl HqTool for GithubCloneTool {
    fn name(&self) -> &str {
        "github_clone"
    }

    fn description(&self) -> &str {
        "Shallow-clone a GitHub repo (owner/name) into a scratch folder and return its path, so it can be \
         searched with grep and read_file. Read-only use: the checkout is for reading, not for committing or pushing."
    }

    fn category(&self) -> &str {
        "web"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"repo": {"type": "string", "description": "owner/name"}},
            "required": ["repo"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let repo = repo(&args)?;
        let dest = clone_root().join(&repo);
        if dest.exists() {
            return Ok(json!({ "path": dest.display().to_string(), "cached": true }));
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let dest_arg = dest.display().to_string();
        let have_gh = which::which("gh").is_ok();
        if have_gh {
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
